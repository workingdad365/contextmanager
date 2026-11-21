#![cfg_attr(not(test), windows_subsystem = "windows")]

mod action;
mod backup;
mod config;
mod i18n;
mod model;
mod path_util;
mod reg;
mod scanner;
mod system;
mod ui;
mod update;

use std::path::PathBuf;

use i18n::Language;
use ui::LaunchOptions;

fn parse_launch_options() -> LaunchOptions {
    let mut opts = LaunchOptions::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lang" => {
                if let Some(code) = args.next() {
                    opts.lang_override = Language::from_code(&code);
                }
            }
            "--ready-file" => {
                if let Some(path) = args.next() {
                    opts.ready_file = Some(PathBuf::from(path));
                }
            }
            _ => {}
        }
    }
    opts
}

fn main() {
    let opts = parse_launch_options();

    // 스크린샷 자동 촬영(--ready-file) 모드가 아닌 일반 실행에서는 시작 시 관리자 권한 확보
    if opts.ready_file.is_none() && !system::is_elevated() {
        let already_retried = std::env::args().any(|a| a == system::ELEVATED_ARG);
        if !already_retried && system::relaunch_as_admin().is_ok() {
            return;
        }
        return;
    }

    ui::run(opts);
}
