# 📋 AGENT GUIDELINES & PROJECT RULES

> **Phiên bản hiện tại của dự án:** `0.0.0-preview`

Tài liệu này định nghĩa các nguyên tắc phát triển và quy định bắt buộc đối với AI Agent và lập trình viên khi làm việc trong dự án **GhitaPrint**:

---

## 🚨 QUY TẮC QUAN TRỌNG NHẤT (TỐI THƯỢNG)
* **Tuyệt đối KHÔNG TỰ Ý `git commit` hay `git push` khi chưa có sự cho phép cụ thể từ người dùng (áp dụng cho cả Git Local và GitHub/Remote repository).**
* Mọi thao tác commit hay push đều phải được người dùng trực tiếp yêu cầu hoặc xác nhận rõ ràng mới được thực hiện.
* Không được tự động commit ngầm sau khi thực hiện xong task hay sau khi hoàn thành các lệnh.

---

### 1. Không có ghi chú trong mã nguồn (Zero Code Comments)
* Trong mã nguồn tuyệt đối không bao giờ được viết thêm ghi chú (`// ...`, `/// ...`, `/* ... */`).
* Mã nguồn phải được cấu trúc sạch sẽ, rõ ràng, đặt tên biến/hàm tường minh (self-documenting code).

### 2. Định dạng Commit Message theo số phiên bản (Version-Only Commits)
* Khi được người dùng cho phép commit, nội dung commit bắt buộc phải là số phiên bản chuẩn Semantic Versioning: `x.x.x` hoặc `x.x.x-tag`.
* **Ví dụ chuẩn:** `0.0.0-preview`, `0.0.1`, `0.1.0`, `1.0.0`...
* Tuyệt đối không đặt commit message tùy tiện hoặc chứa câu mô tả dài dòng.

### 3. Quy chuẩn đặt tên file Setup phát hành trong thư mục `release/`
* Mọi file bộ cài đặt phát hành nằm trong thư mục `d:\GhitaPrint\release` bắt buộc phải tuân theo định dạng:
  `GhitaPrint_x.x.x(-beta/-alpha/...).exe`
* **Ví dụ tương ứng phiên bản hiện tại:** `GhitaPrint_0.0.0-preview.exe`
* Thư mục `release/` chỉ chứa duy nhất một file cài đặt này cho phiên bản phát hành hiện tại.

### 4. Báo cáo trước khi chỉnh sửa `AGENT.md` và `README.md`
* Bất kỳ thay đổi hoặc chỉnh sửa nào đối với hai tệp tin `AGENT.md` và `README.md` đều bắt buộc phải báo cáo và được sự đồng ý/xác nhận từ người dùng trước khi thực hiện.

### 5. Bảo mật tài liệu nội bộ và cách ly qua `.gitignore`
* Mọi tài liệu nội bộ, thông tin riêng tư, bản nháp, file rác và tài liệu không công khai bắt buộc phải được đưa vào `.gitignore`.
* Thư mục `d:\GhitaPrint\docs` được quy định dành riêng để lưu trữ các tài liệu nội bộ không công khai và bắt buộc phải luôn nằm trong danh sách `.gitignore`.
