fn main() {
    // 模型文件变更时强制重跑 build script，触发 resources 复制
    println!("cargo:rerun-if-changed=models/builtin");
    tauri_build::build()
}
