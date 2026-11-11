# 构建与打包

## 环境要求

- Node.js（Vite 8 要求 20.19+ 或 22.12+，建议使用当前 LTS；开发环境为 v22）
- Rust stable + MSVC 工具链（当前依赖要求 Rust 1.95+，建议直接使用最新 stable）
- Visual Studio Build Tools 的 C++ 工作负载 + Windows SDK
- WebView2 Runtime（Windows 11 已内置，Windows 10 可能需要单独安装）

## 开发

```bash
npm install
npm run dev:desktop    # 前端热更新 + 桌面窗口（等同 npm run tauri dev）
```

提交前可运行与 CI 相同的检查：

```bash
npm run lint
npm run format:check
npm test
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml
```

## 浏览器预览 UI

调试布局、配色、字号时不必每次都看 Tauri 窗口：

```bash
npm run dev:web
# 浏览器打开 http://localhost:1420（等同 npm run dev）
```

- 非 Tauri 环境自动加载 `src/mock-sessions.ts` 的模拟数据，覆盖四态、未知状态兜底与死进程残留。
- 页面按 320px 宽度居中显示（隐藏缩放热区），标题栏带「浏览器预览」标记。
- 仅 Tauri 提供的能力（拖拽、关闭、托盘、快捷键、双击激活 VSCode / Git Bash）在浏览器中会以提示代替。
- 开发环境可用 `http://localhost:1420/?settings=1` 直接打开设置面板。
- Vite 固定使用 1420 端口（`strictPort`），端口被占用时启动会失败。

## 打包安装包

1. 先退出正在运行的 Pinglet（托盘右键 → 退出），否则 release 下的 `pinglet.exe` 被占用，构建会报 `os error 5`。
2. 在仓库根目录执行：

   ```bash
   npm install                          # 仅首次或依赖有变化时
   npm run build:nsis                   # 等同 npm run tauri build -- --bundles nsis
   ```

3. 产物：
   - 安装包：`src-tauri/target/release/bundle/nsis/Pinglet_<version>_x64-setup.exe`
   - 便携版：`src-tauri/target/release/pinglet.exe`

NSIS 工具集缓存在 `%LOCALAPPDATA%\tauri\NSIS`，缓存存在时打包不再访问 GitHub。若缓存被清理且 GitHub 直连超时，可手动下载 `nsis-3.13.zip`（SHA1 `db14f8af2ea346b786c6ae19343f61aaf5e09b8e`）和 `nsis_tauri_utils.dll`（SHA1 `75197fee3c6a814fe035788d1c34ead39349b860`）放入该目录，再重新执行构建。

## 图标

- 应用 / 托盘 / 安装包图标以 `assets/app-icon.svg` 为唯一源文件（1024×1024，透明背景，与顶栏 logo 同图形同配色，图形放大约 1.7 倍并加粗）。
- 修改后运行 `npx tauri icon assets/app-icon.svg` 重新生成 `src-tauri/icons/`。
- 仓库只保留 `bundle.icon` 引用的 5 个文件（`32x32.png`、`128x128.png`、`128x128@2x.png`、`icon.ico`、`icon.icns`），其余尺寸、Store 图标与 `android/`、`ios/` 目录在生成后删除。托盘三态图标由 Rust 在运行时基于 exe 内嵌的默认窗口图标合成：先叠加深绿空心圆角描边生成托盘专用底图（`build_tray_badge_icon`），再生成帧（空闲单帧、进行中透明度呼吸 + 轻微缩放帧、告警隐藏 / 显示闪烁帧），无需单独素材或配置；生成物不要手工编辑。
- `build.rs` 已声明 `cargo:rerun-if-changed=icons`，图标重新生成后会触发 Windows 资源重编。若 debug 版托盘仍显示旧图标，重启 `tauri dev`，必要时执行 `cargo clean -p pinglet` 后再构建。
- 顶栏 logo 是 `index.html` 内的内联 SVG，几何与配色需与 `assets/app-icon.svg` 保持一致。

## 版本号

版本号在三处保持同步：

- `package.json`
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.toml`

Vite 从 `package.json` 读取版本并注入 `__APP_VERSION__`，设置面板「关于」区展示该值。

## CI 与发布

- `.github/workflows/ci.yml`：推送到 `master`、提交 PR 或手动触发时，在 `windows-latest` 上执行 ESLint、Prettier 校验、Vitest、前端构建、`cargo fmt --check` 与 `cargo test`。先构建前端再跑 Rust 测试，因为 `generate_context!` 需要 `dist/`。
- `.github/workflows/release.yml`：推送 `v*` 标签或手动触发时，构建 NSIS 安装包并创建**草稿** Release，同时上传 `Pinglet_<version>_x64-portable.zip`（内含 `pinglet.exe`）。检查草稿内容后再手动发布。

发布新版本：

1. 同步 `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 的版本号。
2. 提交并推送 `master`，确认 CI 通过。
3. 打标签并推送：`git tag v1.4.1`、`git push origin v1.4.1`。
4. 在 GitHub Actions 等待 Release 工作流完成，检查草稿 Release 的安装包与便携版后发布。

> 如果目标版本的标签已存在，先递增版本号（例如 `1.4.1`）并同步三处版本，再打新标签；不要重复推送同一个标签。

### 在 GitHub 页面发布

自动流程创建的 Release 默认是**草稿**，需要手动发布：

1. 打开 <https://github.com/fongzhizhi/pinglet/releases>，找到对应版本的草稿。
2. 确认 Assets 包含 `Pinglet_<version>_x64-setup.exe` 和 `Pinglet_<version>_x64-portable.zip`。
3. 按需补充发布说明，点击 **Publish release**。

如果要手动上传本地构建的安装包：

1. 在 Releases 页面点击 **Draft a new release**，选择或创建 `v<version>` 标签，目标分支选 `master`。
2. 标题填 `Pinglet v<version>`，说明按需填写。
3. 把 `src-tauri/target/release/bundle/nsis/Pinglet_<version>_x64-setup.exe` 和便携版 zip 拖入 Assets 区域，或点击 **Attach binaries** 选择文件。
4. 确认无误后点击 **Publish release**；如果只是预告，可以勾选 **Set as a pre-release** 或先保存为草稿。

> `release.yml` 监听 `v*` 标签。手动在网页创建 `v<version>` 标签也会触发自动构建；建议优先使用自动生成的草稿，避免同一版本出现两个 Release。

当前安装包未配置代码签名，首次运行可能触发 Windows SmartScreen 提示。
