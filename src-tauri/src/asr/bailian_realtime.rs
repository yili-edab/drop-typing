//! 阿里百炼 fun-asr-realtime 适配器（DashScope 原生 WebSocket 流式协议）。
//!
//! 协议（已用真实 key 实测验证）：
//! - URL：`wss://<host>/api-ws/v1/inference`（配置的 compatible-mode base_url 会自动推导）
//! - Header：`Authorization: bearer <key>`
//! - 客户端 → 服务端：
//!   1. `run-task`（JSON text）：task_group=audio / task=asr / function=recognition，
//!      parameters: sample_rate=16000, format=pcm
//!   2. 二进制音频帧（PCM s16le 16kHz mono，任意大小 chunk）
//!   3. `finish-task`（JSON text）：标记输入结束
//! - 服务端 → 客户端：
//!   - `task-started`：会话建立，可以开始送音频
//!   - `result-generated`：payload.output.sentence.{text, sentence_end}（text 句内累积）
//!   - `task-finished` / `task-failed`

use std::sync::mpsc as std_mpsc;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc as tokio_mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use super::{RealtimeAsrProvider, RealtimeSession, FINISH_TIMEOUT};

const DEFAULT_WS_URL: &str = "wss://dashscope.aliyuncs.com/api-ws/v1/inference";
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

pub struct BailianRealtimeAsr {
    api_key: String,
    model: String,
    ws_url: String,
    /// Option 以便 Drop 时移出到其他线程销毁
    /// （tokio Runtime 不能在异步上下文中 drop，example / 测试场景会踩到）
    runtime: Option<tokio::runtime::Runtime>,
}

impl BailianRealtimeAsr {
    pub fn new(api_key: String, model: String, base_url: Option<&str>) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("drop-typing-asr-ws")
            .build()?;
        Ok(Self {
            api_key,
            model,
            ws_url: derive_ws_url(base_url),
            runtime: Some(runtime),
        })
    }

    /// 从配置的 base_url 推导 WebSocket 推理端点：
    /// - http(s):// → ws(s)://
    /// - `.../compatible-mode/v1` → `.../api-ws/v1/inference`
    /// - 已是 api-ws 路径则原样使用
    pub fn derive_ws_url(base_url: Option<&str>) -> String {
        derive_ws_url(base_url)
    }
}

fn derive_ws_url(base_url: Option<&str>) -> String {
    let Some(b) = base_url.map(|s| s.trim()).filter(|s| !s.is_empty()) else {
        return DEFAULT_WS_URL.to_string();
    };
    let mut u = b.to_string();
    if u.starts_with("http") {
        u = u.replacen("http", "ws", 1); // https→wss, http→ws
    }
    if u.contains("/compatible-mode/") {
        u = u.replace("/compatible-mode/v1", "/api-ws/v1/inference");
    }
    u.trim_end_matches('/').to_string()
}

enum ClientMsg {
    Audio(Vec<u8>),
    Finish,
}

impl Drop for BailianRealtimeAsr {
    fn drop(&mut self) {
        // tokio Runtime 不能在异步上下文中 drop（会 panic），移到独立线程销毁
        if let Some(rt) = self.runtime.take() {
            std::thread::spawn(move || drop(rt));
        }
    }
}

struct BailianSession {
    tx: tokio_mpsc::UnboundedSender<ClientMsg>,
    result_rx: Mutex<std_mpsc::Receiver<Result<String, String>>>,
}

impl RealtimeSession for BailianSession {
    fn send_audio(&self, pcm: &[u8]) -> Result<()> {
        self.tx
            .send(ClientMsg::Audio(pcm.to_vec()))
            .map_err(|_| anyhow!("ASR 会话已关闭"))
    }

    fn finish(&self) -> Result<String> {
        let _ = self.tx.send(ClientMsg::Finish);
        let rx = self.result_rx.lock().unwrap();
        match rx.recv_timeout(FINISH_TIMEOUT) {
            Ok(Ok(text)) => Ok(text),
            Ok(Err(e)) => Err(anyhow!(e)),
            Err(_) => Err(anyhow!("识别超时（{}s 无服务端响应）", FINISH_TIMEOUT.as_secs())),
        }
    }
}

impl RealtimeAsrProvider for BailianRealtimeAsr {
    fn start_session(
        &self,
        partial_tx: std_mpsc::Sender<String>,
    ) -> Result<Box<dyn RealtimeSession>> {
        let (tx, rx) = tokio_mpsc::unbounded_channel::<ClientMsg>();
        let (result_tx, result_rx) = std_mpsc::channel::<Result<String, String>>();
        let (ready_tx, ready_rx) = std_mpsc::channel::<Result<(), String>>();

        let (url, key, model) = (
            self.ws_url.clone(),
            self.api_key.clone(),
            self.model.clone(),
        );
        let url_for_err = url.clone();
        self.runtime
            .as_ref()
            .ok_or_else(|| anyhow!("ASR runtime 已销毁"))?
            .spawn(run_connection(
                url, key, model, rx, partial_tx, result_tx, ready_tx,
            ));

        // 等待 task-started 或快速失败，让"按下"瞬间就能暴露连接/鉴权问题
        match ready_rx.recv_timeout(CONNECT_TIMEOUT) {
            Ok(Ok(())) => Ok(Box::new(BailianSession {
                tx,
                result_rx: Mutex::new(result_rx),
            })),
            Ok(Err(e)) => Err(anyhow!(e)),
            Err(_) => Err(anyhow!(
                "连接 ASR 服务超时（{}s）：{url_for_err}",
                CONNECT_TIMEOUT.as_secs()
            )),
        }
    }
}

fn run_task_message(task_id: &str, model: &str) -> String {
    json!({
        "header": {
            "action": "run-task",
            "task_id": task_id,
            "streaming": "duplex"
        },
        "payload": {
            "task_group": "audio",
            "task": "asr",
            "function": "recognition",
            "model": model,
            "parameters": {
                "sample_rate": 16000,
                "format": "pcm"
            },
            "input": {}
        }
    })
    .to_string()
}

fn finish_task_message(task_id: &str) -> String {
    json!({
        "header": {
            "action": "finish-task",
            "task_id": task_id,
            "streaming": "duplex"
        },
        "payload": { "input": {} }
    })
    .to_string()
}

async fn run_connection(
    url: String,
    api_key: String,
    model: String,
    mut rx: tokio_mpsc::UnboundedReceiver<ClientMsg>,
    partial_tx: std_mpsc::Sender<String>,
    result_tx: std_mpsc::Sender<Result<String, String>>,
    ready_tx: std_mpsc::Sender<Result<(), String>>,
) {
    // ---- 连接 + 鉴权 ----
    let request = match url.as_str().into_client_request() {
        Ok(mut r) => {
            match HeaderValue::from_str(&format!("bearer {api_key}")) {
                Ok(v) => {
                    r.headers_mut().insert("Authorization", v);
                    r
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("API Key 非法：{e}")));
                    return;
                }
            }
        }
        Err(e) => {
            let _ = ready_tx.send(Err(format!("WebSocket URL 非法（{url}）：{e}")));
            return;
        }
    };
    let (ws, _) = match tokio_tungstenite::connect_async(request).await {
        Ok(x) => x,
        Err(e) => {
            let _ = ready_tx.send(Err(format!("WebSocket 连接失败（{url}）：{e}")));
            return;
        }
    };
    let (mut write, mut read) = ws.split();

    // ---- 下发 run-task ----
    let task_id = uuid::Uuid::new_v4().simple().to_string();
    if let Err(e) = write
        .send(Message::Text(run_task_message(&task_id, &model).into()))
        .await
    {
        let _ = ready_tx.send(Err(format!("run-task 发送失败：{e}")));
        return;
    }

    // ---- 主循环：读写复用 ----
    let mut committed = String::new(); // 已定稿句子拼接
    let mut current = String::new(); // 当前句中间结果
    let mut ready_sent = false;
    let mut finish_sent = false;

    macro_rules! fail {
        ($msg:expr) => {{
            let m: String = $msg;
            if !ready_sent {
                let _ = ready_tx.send(Err(m));
            } else {
                let _ = result_tx.send(Err(m));
            }
            return;
        }};
    }

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(txt))) => {
                        let v: Value = match serde_json::from_str(&txt) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        match v["header"]["event"].as_str().unwrap_or("") {
                            "task-started" => {
                                ready_sent = true;
                                let _ = ready_tx.send(Ok(()));
                            }
                            "result-generated" => {
                                let sentence = &v["payload"]["output"]["sentence"];
                                let text = sentence["text"].as_str().unwrap_or("").to_string();
                                let sentence_end =
                                    sentence["sentence_end"].as_bool().unwrap_or(false);
                                current = text;
                                if sentence_end {
                                    committed.push_str(&current);
                                    current.clear();
                                }
                                // 累积全文推给前端做中间展示
                                let _ = partial_tx.send(format!("{committed}{current}"));
                            }
                            "task-finished" => {
                                committed.push_str(&current);
                                let _ = result_tx.send(Ok(committed));
                                return;
                            }
                            "task-failed" => {
                                let err = v["header"]["error_message"]
                                    .as_str()
                                    .unwrap_or("未知错误")
                                    .to_string();
                                fail!(format!("ASR 任务失败：{err}"));
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        fail!("ASR WebSocket 连接被关闭".to_string());
                    }
                    Some(Err(e)) => {
                        fail!(format!("ASR WebSocket 错误：{e}"));
                    }
                    _ => {} // Binary / Ping / Pong
                }
            }
            cmd = rx.recv(), if !finish_sent => {
                match cmd {
                    Some(ClientMsg::Audio(pcm)) => {
                        if write.send(Message::Binary(pcm.into())).await.is_err() {
                            fail!("音频发送失败（连接已断开）".to_string());
                        }
                    }
                    Some(ClientMsg::Finish) | None => {
                        let _ = write
                            .send(Message::Text(finish_task_message(&task_id).into()))
                            .await;
                        finish_sent = true; // 关闭该分支，等待 task-finished
                    }
                }
            }
        }
    }
}
