# iOS Location (MyGo)

基于 [MyGo](https://github.com/egoist/mygo)（纯 Go、`CGO_ENABLED=0`）实现的 macOS 原生 iOS GPS 模拟定位桌面应用，支持 **iOS 17+ / 27+ 无线直连定位模拟（免 `sudo`、独立 RemotePairing 隧道）** 与 USB 有线 `xcrun devicectl` 模式。

## 特性
- **纯 Go 原生桌面体验**：macOS 原生 `WKWebView` + MyGo Typed IPC + Apple Liquid Glass 浮层 UI（浅色 / 深色自动跟随并支持手动切换），无 Electron/Node 运行时依赖。
- **三通道设备发现与内置无线配对**：
  - 独立 **iOS 17+ / 27+ RemotePairing 隧道**（`pymobiledevice3 remote browse` / `--remotepairing`），摆脱 macOS `usbmuxd` 空闲掉线问题。
  - 内置 **Wi-Fi 无线配对向导**（`pymobiledevice3 remote pair-host`），支持 6 位 PIN 码局域网直连配对。
  - 自动兼容 `usbmux` 与 Xcode `xcrun devicectl` 有线模式，单隧道平滑切换与真实 GPS 一键恢复。
- **多图源与自动纠偏**：内置 **Google Maps / Google Satellite**、Esri 街道、高德地图、高德卫星，自动完成中国大陆 WGS-84 ↔ GCJ-02 坐标转换。
- **中英双语 i18n（`zh-Hans` / `en`）**：内置一键语言切换，实时同步前端界面、Go 后端日志/状态、Google 地图瓦片语言及 macOS 原生菜单栏，并持久化语言偏好。
- **离线收藏与隐私安全**：收藏夹保存在本地 `~/Library/Application Support/iOS Location/favorites.json`。

## 前置依赖
- macOS（Apple Silicon / Intel）
- Go 1.27.3+（`GOTOOLCHAIN=auto` 会自动拉取）
- [`pymobiledevice3`](https://github.com/doronz88/pymobiledevice3)：`uv tool install pymobiledevice3`
- iPhone 已开启「开发者模式」（无线模式需与 Mac 处于同一 Wi-Fi，可在应用内点击 Wi-Fi 配对图标完成无线配对）

## 开发、测试与打包

```bash
# 运行单元测试
CGO_ENABLED=0 go test -count=1 ./...

# 更新多语言访问器代码（修改 locales/*.json 后）
go run github.com/egoist/mygo/cmd/mygo@v0.10.0 generate .

# 本地直接运行
go run .

# 打包 macOS .app 与 .dmg 安装包（自动生成 AppIcon.icns 与 .lproj 本地化包）
go run github.com/egoist/mygo/cmd/mygo@v0.10.0 build .
```
