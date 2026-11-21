# ContextManager

Windows 10 / 11 탐색기의 마우스 오른쪽 버튼 컨텍스트 메뉴 항목을 조회하고, 선택적으로 비활성화(숨기기)하거나 자동 백업 후 제거하는 데스크톱 유틸리티.

## 주요 기능

- **통합 스캔**:
  - 정적/동적 Shell 명령 (`HKCU`/`HKLM`의 `*\shell`, `Directory\shell`, `Directory\Background\shell`, `Folder`, `Drive`, `DesktopBackground`, `SystemFileAssociations`, 확장자/ProgID 전체)
  - 클래식 COM 셸 확장 (`shellex\ContextMenuHandlers`)
  - Windows 11 신형 컨텍스트 메뉴 (MSIX/Sparse Package `IExplorerCommand`)
  - 보내기 (`SendTo`)
  - 새로 만들기 (`ShellNew`)
- **앱별 묶어 보기 및 일괄 정리**: 실행 파일/DLL 경로와 표시 이름이 같은 여러 레지스트리 위치를 하나의 그룹으로 묶어 일괄 선택 및 제거 지원
- **잔여 항목 자동 탐지**: 실행 파일이나 DLL이 디스크에서 이미 삭제된 항목을 `[파일 없음 - 잔여 항목]`으로 자동 강조
- **임시 비활성화 (숨기기) 및 다시 켜기**:
  - `Shell 명령`: `LegacyDisable` 값 토글
  - `COM 셸 확장` / `Win11 신형`: `Shell Extensions\Blocked` 등록/해제
  - `보내기`: `%LOCALAPPDATA%\ContextManager\disabled_sendto`로 이동/복원
  - `새로 만들기`: `ShellNew` <-> `ShellNew_Disabled` 이름 변경
- **자동 백업 및 원클릭 복원**: 항목 제거 전 `%LOCALAPPDATA%\ContextManager\backups`에 JSON 메타데이터와 `.reg` 파일을 자동 저장하고, 프로그램 내 `백업 / 복원` 창에서 바로 복원 가능
- **관리자 권한 자동 실행 및 탐색기 연동**: 실행 시 관리자 권한(UAC)으로 시작하여 `HKCU`와 `HKLM` 항목을 모두 즉시 비활성화/제거할 수 있으며, 레지스트리 편집기(`regedit.exe`) 바로 열기와 탐색기 다시 시작 지원

## 빌드 및 테스트

```powershell
# 유닛 및 통합 테스트 실행
cargo test

# 릴리스 빌드
cargo build --release
```

릴리스 실행 파일은 `target\release\ContextManager.exe`에 생성됨.
