use super::pwg;
use super::routes::AppState;
use crate::encoder::FloydSteinbergRasterizer;
use crate::server::execute_print;
use image::DynamicImage;

pub const OP_PRINT_JOB: u16 = 0x0002;
pub const OP_VALIDATE_JOB: u16 = 0x0004;
pub const OP_GET_PRINTER_ATTRIBUTES: u16 = 0x000A;
pub const OP_CREATE_JOB: u16 = 0x0005;
pub const OP_SEND_DOCUMENT: u16 = 0x0006;
pub const OP_CANCEL_JOB: u16 = 0x0008;

pub const STATUS_OK: u16 = 0x0000;
pub const STATUS_CLIENT_BAD_REQUEST: u16 = 0x0400;
pub const STATUS_CLIENT_FORMAT_NOT_SUPPORTED: u16 = 0x0406;
pub const STATUS_SERVER_INTERNAL: u16 = 0x0500;
pub const STATUS_SERVER_OP_NOT_SUPPORTED: u16 = 0x0501;

pub const TAG_OPERATION_ATTRIBUTES: u8 = 0x01;
pub const TAG_JOB_ATTRIBUTES: u8 = 0x02;
pub const TAG_END_OF_ATTRIBUTES: u8 = 0x03;
pub const TAG_PRINTER_ATTRIBUTES: u8 = 0x04;

pub const TAG_INTEGER: u8 = 0x21;
pub const TAG_BOOLEAN: u8 = 0x22;
pub const TAG_ENUM: u8 = 0x23;
pub const TAG_TEXT: u8 = 0x41;
pub const TAG_NAME: u8 = 0x42;
pub const TAG_KEYWORD: u8 = 0x44;
pub const TAG_URI: u8 = 0x45;
pub const TAG_CHARSET: u8 = 0x47;
pub const TAG_NATURAL_LANGUAGE: u8 = 0x48;
pub const TAG_MIME: u8 = 0x49;
pub const TAG_RESOLUTION: u8 = 0x35;

pub const JOB_STATE_PENDING: i32 = 3;
pub const JOB_STATE_PROCESSING: i32 = 5;
pub const JOB_STATE_ABORTED: i32 = 8;
pub const JOB_STATE_COMPLETED: i32 = 9;

#[derive(Debug, Clone)]
pub struct IppAttribute {
    pub tag: u8,
    pub name: String,
    pub value: Vec<u8>,
}

#[derive(Debug)]
pub struct IppRequest {
    pub version: (u8, u8),
    pub operation: u16,
    pub request_id: u32,
    pub groups: Vec<(u8, Vec<IppAttribute>)>,
    pub data: Vec<u8>,
}

impl IppRequest {
    pub fn string_value(&self, name: &str) -> Option<String> {
        for (_, attrs) in &self.groups {
            for attr in attrs {
                if attr.name == name {
                    return String::from_utf8(attr.value.clone()).ok();
                }
            }
        }
        None
    }
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let b = data.get(offset..offset + 2)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let b = data.get(offset..offset + 4)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

pub fn parse_ipp(data: &[u8]) -> Result<IppRequest, String> {
    if data.len() < 8 {
        return Err("IPP: dữ liệu ngắn hơn 8 bytes header".to_string());
    }

    let version = (data[0], data[1]);
    let operation = read_u16(data, 2).ok_or("IPP: thiếu operation-id")?;
    let request_id = read_u32(data, 4).ok_or("IPP: thiếu request-id")?;

    let mut offset = 8usize;
    let mut groups: Vec<(u8, Vec<IppAttribute>)> = Vec::new();
    let mut current_group: u8 = 0;
    let mut current_attrs: Vec<IppAttribute> = Vec::new();
    let mut payload_start: Option<usize> = None;

    while offset < data.len() {
        let tag = data[offset];
        offset += 1;

        if tag == TAG_END_OF_ATTRIBUTES {
            payload_start = Some(offset);
            break;
        }

        if tag <= 0x0F {
            if current_group != 0 || !current_attrs.is_empty() {
                groups.push((current_group, std::mem::take(&mut current_attrs)));
            }
            current_group = tag;
            continue;
        }

        let name_len = read_u16(data, offset).ok_or("IPP: thiếu name-length")? as usize;
        offset += 2;
        let name = if name_len == 0 {
            String::new()
        } else {
            let raw = data
                .get(offset..offset + name_len)
                .ok_or("IPP: name vượt quá dữ liệu")?;
            offset += name_len;
            String::from_utf8_lossy(raw).to_string()
        };

        let value_len = read_u16(data, offset).ok_or("IPP: thiếu value-length")? as usize;
        offset += 2;
        let value = data
            .get(offset..offset + value_len)
            .ok_or("IPP: value vượt quá dữ liệu")?
            .to_vec();
        offset += value_len;

        if name.is_empty()
            && let Some(last) = current_attrs.last_mut()
        {
            last.value.extend_from_slice(&value);
            continue;
        }

        current_attrs.push(IppAttribute {
            tag,
            name: name.to_string(),
            value,
        });
    }

    if current_group != 0 || !current_attrs.is_empty() {
        groups.push((current_group, current_attrs));
    }

    let document = match payload_start {
        Some(start) if start <= data.len() => data[start..].to_vec(),
        _ => Vec::new(),
    };

    Ok(IppRequest {
        version,
        operation,
        request_id,
        groups,
        data: document,
    })
}

fn push_attr(out: &mut Vec<u8>, tag: u8, name: &str, value: &[u8]) {
    out.push(tag);
    out.extend_from_slice(&(name.len() as u16).to_be_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value);
}

fn push_string(out: &mut Vec<u8>, tag: u8, name: &str, value: &str) {
    push_attr(out, tag, name, value.as_bytes());
}

fn push_integer(out: &mut Vec<u8>, tag: u8, name: &str, value: i32) {
    push_attr(out, tag, name, &value.to_be_bytes());
}

fn push_multi_string(out: &mut Vec<u8>, tag: u8, name: &str, values: &[&str]) {
    for (index, value) in values.iter().enumerate() {
        if index == 0 {
            push_string(out, tag, name, value);
        } else {
            push_attr(out, tag, "", value.as_bytes());
        }
    }
}

fn push_multi_integer(out: &mut Vec<u8>, tag: u8, name: &str, values: &[i32]) {
    for (index, value) in values.iter().enumerate() {
        let encoded = value.to_be_bytes();
        if index == 0 {
            push_attr(out, tag, name, &encoded);
        } else {
            push_attr(out, tag, "", &encoded);
        }
    }
}

fn push_resolution(out: &mut Vec<u8>, name: &str, dpi: i32) {
    let mut value = Vec::with_capacity(13);
    value.extend_from_slice(&dpi.to_be_bytes());
    value.extend_from_slice(&dpi.to_be_bytes());
    value.push(3);
    value.extend_from_slice(&[0, 0, 0, 0]);
    push_attr(out, TAG_RESOLUTION, name, &value);
}

pub struct IppResponseSpec {
    pub status: u16,
    pub job_id: Option<i32>,
    pub job_state: Option<i32>,
    pub job_message: Option<String>,
}

pub fn build_response(
    version: (u8, u8),
    request_id: u32,
    spec: &IppResponseSpec,
    include_printer_attributes: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(&[version.0, version.1]);
    out.extend_from_slice(&spec.status.to_be_bytes());
    out.extend_from_slice(&request_id.to_be_bytes());

    out.push(TAG_OPERATION_ATTRIBUTES);
    push_string(&mut out, TAG_CHARSET, "attributes-charset", "utf-8");
    push_string(
        &mut out,
        TAG_NATURAL_LANGUAGE,
        "attributes-natural-language",
        "en",
    );
    if let Some(job_id) = spec.job_id {
        push_integer(&mut out, TAG_INTEGER, "job-id", job_id);
    }
    if let Some(job_state) = spec.job_state {
        push_integer(&mut out, TAG_ENUM, "job-state", job_state);
    }
    if let Some(message) = &spec.job_message {
        push_string(&mut out, TAG_TEXT, "status-message", message);
    }

    if include_printer_attributes {
        out.push(TAG_PRINTER_ATTRIBUTES);
        push_string(
            &mut out,
            TAG_URI,
            "printer-uri-supported",
            "ipp://127.0.0.1/printers/ghitaprint",
        );
        push_integer(&mut out, TAG_ENUM, "printer-state", 3);
        push_string(&mut out, TAG_KEYWORD, "printer-state-reasons", "none");
        push_string(
            &mut out,
            TAG_TEXT,
            "printer-make-and-model",
            "GhitaPrint Virtual Printer",
        );
        push_multi_string(
            &mut out,
            TAG_KEYWORD,
            "ipp-versions-supported",
            &["1.1", "2.0"],
        );
        push_multi_integer(
            &mut out,
            TAG_ENUM,
            "operations-supported",
            &[
                OP_PRINT_JOB as i32,
                OP_VALIDATE_JOB as i32,
                OP_CREATE_JOB as i32,
                OP_SEND_DOCUMENT as i32,
                OP_CANCEL_JOB as i32,
                OP_GET_PRINTER_ATTRIBUTES as i32,
            ],
        );
        push_multi_string(
            &mut out,
            TAG_MIME,
            "document-format-supported",
            &["image/pwg-raster", "application/octet-stream", "text/plain"],
        );
        push_multi_string(
            &mut out,
            TAG_KEYWORD,
            "compression-supported",
            &["none", "flate"],
        );
        push_multi_string(
            &mut out,
            TAG_KEYWORD,
            "pwg-raster-document-type-supported",
            &["sgray_8", "srgb_8"],
        );
        push_resolution(&mut out, "pwg-raster-document-resolution-supported", 600);
        push_resolution(&mut out, "pwg-raster-document-resolution-supported", 300);
    }

    out.push(TAG_END_OF_ATTRIBUTES);
    out
}

fn page_to_dynamic_image(page: &pwg::PwgPage) -> Result<DynamicImage, String> {
    image::RgbImage::from_raw(page.width, page.height, page.rgba.clone())
        .map(DynamicImage::ImageRgb8)
        .ok_or_else(|| {
            format!(
                "PWG: kích thước trang {}x{} không khớp buffer {} bytes",
                page.width,
                page.height,
                page.rgba.len()
            )
        })
}

fn render_pages_to_escpos(pages: &[pwg::PwgPage]) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new();
    let mut first = true;
    for page in pages {
        let image = page_to_dynamic_image(page)?;
        let raster = FloydSteinbergRasterizer::to_escpos_raster(&image, None)?;
        if first {
            out.extend_from_slice(&[0x1B, 0x40]);
            first = false;
        }
        out.extend_from_slice(&raster);
        out.extend_from_slice(&[0x1B, 0x64, 0x01]);
    }
    if out.is_empty() {
        return Err("Không render được trang nào ra raster ESC/POS".to_string());
    }
    Ok(out)
}

pub async fn handle_ipp_request(state: &AppState, body: &[u8]) -> (Vec<u8>, String) {
    let request = match parse_ipp(body) {
        Ok(req) => req,
        Err(e) => {
            let response = build_response(
                (1, 1),
                1,
                &IppResponseSpec {
                    status: STATUS_CLIENT_BAD_REQUEST,
                    job_id: None,
                    job_state: None,
                    job_message: Some(e.clone()),
                },
                false,
            );
            return (response, format!("IPP payload không hợp lệ: {}", e));
        }
    };

    tracing::info!(
        "IPP: operation=0x{:04x} request_id={} data={} bytes",
        request.operation,
        request.request_id,
        request.data.len()
    );

    match request.operation {
        OP_GET_PRINTER_ATTRIBUTES => {
            let response = build_response(
                request.version,
                request.request_id,
                &IppResponseSpec {
                    status: STATUS_OK,
                    job_id: None,
                    job_state: None,
                    job_message: None,
                },
                true,
            );
            (
                response,
                "IPP Get-Printer-Attributes trả attribute".to_string(),
            )
        }
        OP_VALIDATE_JOB => {
            let response = build_response(
                request.version,
                request.request_id,
                &IppResponseSpec {
                    status: STATUS_OK,
                    job_id: None,
                    job_state: Some(JOB_STATE_PENDING),
                    job_message: None,
                },
                false,
            );
            (response, "IPP Validate-Job chấp nhận".to_string())
        }
        OP_PRINT_JOB => print_job(state, &request, None).await,
        OP_CREATE_JOB => {
            let job_id = state.allocate_job_id() as i32;
            let response = build_response(
                request.version,
                request.request_id,
                &IppResponseSpec {
                    status: STATUS_OK,
                    job_id: Some(job_id),
                    job_state: Some(JOB_STATE_PENDING),
                    job_message: None,
                },
                false,
            );
            (response, format!("IPP Create-Job id={}", job_id))
        }
        OP_SEND_DOCUMENT => {
            let job_id = request
                .string_value("job-id")
                .and_then(|s| s.parse::<i32>().ok());
            print_job(state, &request, job_id).await
        }
        OP_CANCEL_JOB => {
            let response = build_response(
                request.version,
                request.request_id,
                &IppResponseSpec {
                    status: STATUS_OK,
                    job_id: None,
                    job_state: Some(7),
                    job_message: Some("Job đã bị hủy".to_string()),
                },
                false,
            );
            (response, "IPP Cancel-Job".to_string())
        }
        other => {
            let response = build_response(
                request.version,
                request.request_id,
                &IppResponseSpec {
                    status: STATUS_SERVER_OP_NOT_SUPPORTED,
                    job_id: None,
                    job_state: None,
                    job_message: Some(format!("Operation 0x{:04x} không được hỗ trợ", other)),
                },
                false,
            );
            (
                response,
                format!("IPP operation 0x{:04x} chưa hỗ trợ", other),
            )
        }
    }
}

async fn print_job(
    state: &AppState,
    request: &IppRequest,
    existing_job_id: Option<i32>,
) -> (Vec<u8>, String) {
    let job_id = existing_job_id.unwrap_or_else(|| state.allocate_job_id() as i32);
    let compression = request.string_value("compression");
    let document_format = request.string_value("document-format");

    let print_bytes = match pwg::decode_pwg_document(&request.data, compression.as_deref()) {
        Ok(pages) => match render_pages_to_escpos(&pages) {
            Ok(bytes) => {
                tracing::info!(
                    "IPP job {}: decode PWG {} trang → {} bytes ESC/POS",
                    job_id,
                    pages.len(),
                    bytes.len()
                );
                Ok(bytes)
            }
            Err(e) => Err(e),
        },
        Err(pwg_error) => {
            if request.data.is_empty() {
                Err(format!(
                    "IPP job {}: không có dữ liệu trang in — {}",
                    job_id, pwg_error
                ))
            } else {
                tracing::warn!(
                    "IPP job {}: PWG decode thất bại ({}), fallback in thô {} bytes (format: {:?})",
                    job_id,
                    pwg_error,
                    request.data.len(),
                    document_format
                );
                Ok(request.data.clone())
            }
        }
    };

    let bytes = match print_bytes {
        Ok(bytes) => bytes,
        Err(e) => {
            let response = build_response(
                request.version,
                request.request_id,
                &IppResponseSpec {
                    status: STATUS_CLIENT_FORMAT_NOT_SUPPORTED,
                    job_id: Some(job_id),
                    job_state: Some(JOB_STATE_ABORTED),
                    job_message: Some(e.clone()),
                },
                false,
            );
            return (response, format!("IPP job {} lỗi decode: {}", job_id, e));
        }
    };

    let outcome = execute_print(state, "ipp", None, &bytes, "ipp").await;
    let (status, job_state, message) = if outcome.success {
        (STATUS_OK, JOB_STATE_COMPLETED, outcome.message.clone())
    } else {
        (
            STATUS_SERVER_INTERNAL,
            JOB_STATE_ABORTED,
            outcome.message.clone(),
        )
    };

    let response = build_response(
        request.version,
        request.request_id,
        &IppResponseSpec {
            status,
            job_id: Some(job_id),
            job_state: Some(job_state),
            job_message: Some(message.clone()),
        },
        false,
    );
    (
        response,
        format!(
            "IPP job {} → {}",
            job_id,
            if outcome.success {
                format!("in thành công ({} bytes)", outcome.bytes_sent)
            } else {
                format!("in thất bại: {}", outcome.message)
            }
        ),
    )
}
