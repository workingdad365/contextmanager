const ADMIN_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;

fn main() {
    println!("cargo:rerun-if-changed=assets/contextmanager.ico");
    println!("cargo:rerun-if-env-changed=PROFILE");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        if std::path::Path::new("assets/contextmanager.ico").exists() {
            res.set_icon_with_id("assets/contextmanager.ico", "1");
        }
        // 릴리스 바이너리에 requireAdministrator 매니페스트 내장
        // (디버그/테스트 바이너리는 cargo test 호환을 유지하고, 런타임 main()에서 권한 상승 처리)
        if std::env::var("PROFILE").as_deref() == Ok("release") {
            res.set_manifest(ADMIN_MANIFEST);
        }
        res.compile().expect("Windows 리소스 컴파일 실패");
    }
}
