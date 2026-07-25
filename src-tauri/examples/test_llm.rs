//! LLM 清洗手动测试入口（不启动 Tauri 应用）。
//!
//! 用法：
//!   cargo run --example test_llm -- "要清洗的文本"
//!   echo "要清洗的文本" | cargo run --example test_llm
//!
//! - 配置读取与 App 一致：~/.drop-typing.toml 的 [llm] 段
//! - 未配置 [llm] 或缺 api_key 时提示退出（此时 App 行为为 ASR 直出）
//! - 依次用 conservative / standard 两档各清洗一遍，方便对比

use drop_typing_lib::config::Config;
use drop_typing_lib::llm::{self, Strength};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let text = match std::env::args().nth(1) {
        Some(t) => t,
        None => {
            use std::io::Read as _;
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf.trim().to_string()
        }
    };
    if text.is_empty() {
        anyhow::bail!("用法: cargo run --example test_llm -- \"要清洗的文本\"（或从 stdin 读入）");
    }
    println!("[test_llm] 原文：{text}");

    let (cfg, warning) = Config::load_lenient();
    if let Some(w) = warning {
        eprintln!("[test_llm] 配置警告：{w}");
    }
    println!(
        "[test_llm] protocol={} model={:?} strength={}",
        cfg.llm_protocol(),
        cfg.llm.model,
        cfg.llm_strength()
    );
    let cleaner = llm::cleaner_from_config(&cfg).ok_or_else(|| {
        anyhow::anyhow!("未配置 [llm] 或缺 api_key（此时 App 行为为 ASR 直出，不清洗）")
    })?;

    for strength in [Strength::Conservative, Strength::Standard] {
        let cleaned = cleaner.clean(&text, strength).await?;
        println!("[test_llm] {strength:?} 清洗结果：{cleaned}");
    }
    Ok(())
}
