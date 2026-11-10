fn main() {
    // NOTE: tauri-build 只为 tauri.conf.json 与 capabilities 声明 rerun-if-changed，
    // 图标文件变化不会触发 Windows 资源重新编译；这里显式声明依赖，
    // 保证 `npx tauri icon` 重新生成 icons/ 后 exe 内嵌图标与托盘图标同步更新。
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
