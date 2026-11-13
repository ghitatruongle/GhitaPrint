use clap::Parser;
use ghitaprint::cli::WatchArgs;

#[derive(Parser, Debug)]
#[command(
    name = "ghitaprint-watch",
    version = env!("CARGO_PKG_VERSION"),
    about = "Ghi nhật ký sự kiện in vào events.log và bắn toast Windows khi in lỗi — companion siêu nhẹ của daemon"
)]
struct WatchCli {
    #[command(flatten)]
    args: WatchArgs,
}

fn main() -> std::process::ExitCode {
    let cli = WatchCli::parse();
    ghitaprint::watch::run_watch(cli.args.interval_ms, cli.args.no_toast, cli.args.once);
    std::process::ExitCode::SUCCESS
}