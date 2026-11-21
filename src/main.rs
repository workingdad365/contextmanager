#![cfg_attr(not(test), windows_subsystem = "windows")]

mod action;
mod backup;
mod model;
mod path_util;
mod reg;
mod scanner;
mod system;
mod ui;

fn main() {
    // 레지스트리(HKLM) 숨기기 및 제거가 항상 동작하도록 시작 시 관리자 권한 확보
    if !system::is_elevated() {
        let already_retried = std::env::args().any(|a| a == system::ELEVATED_ARG);
        if !already_retried && system::relaunch_as_admin().is_ok() {
            return;
        }
        // 사용자가 UAC 승인을 취소했거나 권한 상승에 실패한 경우 종료
        return;
    }

    ui::run();
}
