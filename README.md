# 화면 녹화

시간 제한 없는 Windows 화면 녹화 프로그램 (Tauri 2 + HTML/CSS/JS, 녹화는 ffmpeg gdigrab).

## 기능

- **범위 선택**: 마우스를 올린 창을 클릭하면 그 창 크기로, 드래그하면 원하는 영역으로, 바탕화면을 클릭하면 모니터 전체로 지정
- **모니터 배치도**: 모니터 구성을 그림으로 보여주고, 클릭하면 그 모니터 전체를 녹화 범위로 지정
- **멀티 모니터 · 혼합 DPI** 대응 (물리 픽셀 기준 좌표)
- **녹화에서 제외**: 이 앱 창과 범위 테두리는 영상에 찍히지 않음 (`WDA_EXCLUDEFROMCAPTURE`)
- **전역 단축키**: 기본 F9, 설정에서 원하는 조합으로 변경
- **남은 녹화 가능 시간**: 디스크 여유 공간 기준 예상치, 녹화 중에는 실측 속도로 계산. 공간이 부족하면 자동 정지
- MKV로 녹화한 뒤 정지할 때 MP4로 변환 → 중간에 꺼져도 영상이 남음
- 라이트 / 다크 / 시스템 테마

## 개발

필요: Rust, Node.js, Visual Studio C++ Build Tools, WebView2

```powershell
npm install
powershell -ExecutionPolicy Bypass -File scripts\get-ffmpeg.ps1   # ffmpeg sidecar 받기 (최초 1회)
npm run dev      # 개발 실행
npm run build    # 설치 파일: src-tauri/target/release/bundle/nsis/
```

## 구조

| 경로 | 내용 |
| --- | --- |
| `src/index.html`, `style.css`, `main.js` | 메인 창 |
| `src/selector.html` | 범위 선택 오버레이 (모니터마다 하나씩, Canvas로 그림) |
| `src/border.html` | 선택 범위 테두리 (클릭 통과, 녹화 제외) |
| `src-tauri/src/lib.rs` | 창 목록 · 모니터 · ffmpeg 녹화 제어 · 단축키 |
