//! 中转站 HTTP 客户端：拉模型列表、并发探测 `/chat/completions`。
//!
//! 独立于 egui，是为了 UI 线程只做收消息；网络在 tokio 运行时里跑。
//! 中转站常见坑是用户把完整 chat URL 贴进来，所以 `normalize_base_url` 会剥掉
//! `/models`、`/chat/completions` 这类尾巴，避免拼出 `.../v1/models/models`。
//! 任务用递增 `job` 丢弃过期结果：连点两次时旧请求回来不得覆盖新列表。

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestStatus {
    Pending,
    Testing,
    Pass,
    Fail,
}

#[derive(Debug, Clone)]
pub struct ModelRecord {
    pub id: String,
    pub selected: bool,
    pub status: TestStatus,
    pub latency_ms: Option<u128>,
    pub detail: String,
}

pub enum ServiceMsg {
    ListDone {
        job: u64,
        models: Vec<ModelRecord>,
        log: String,
    },
    TestProgress {
        job: u64,
        models: Vec<ModelRecord>,
    },
    TestDone {
        job: u64,
        models: Vec<ModelRecord>,
        log: String,
    },
}

pub struct ModelService {
    pub base_url: String,
    pub api_key: String,
    running: bool,
    job_id: u64,
    rx: Mutex<Option<mpsc::Receiver<ServiceMsg>>>,
    rt: tokio::runtime::Runtime,
}

impl ModelService {
    pub fn new() -> Self {
        Self {
            base_url: String::new(),
            api_key: String::new(),
            running: false,
            job_id: 0,
            rx: Mutex::new(None),
            rt: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap(),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn current_job(&self) -> u64 {
        self.job_id
    }

    pub fn try_recv(&self) -> Option<ServiceMsg> {
        if let Some(rx) = self.rx.lock().unwrap().as_ref() {
            rx.try_recv().ok()
        } else {
            None
        }
    }

    pub fn set_running(&mut self, v: bool) {
        self.running = v;
    }

    fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default()
    }

    fn auth_headers(&self) -> Result<HeaderMap, String> {
        let mut h = HeaderMap::new();
        let key = self.api_key.trim();
        if key.is_empty() {
            return Err("请填写 Token".into());
        }
        let val = HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|_| "Token 含有非法字符，无法写入 Authorization".to_string())?;
        h.insert(AUTHORIZATION, val);
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(h)
    }

    fn begin_job(&mut self) -> (u64, mpsc::Sender<ServiceMsg>) {
        self.running = true;
        self.job_id = self.job_id.wrapping_add(1);
        let (tx, rx) = mpsc::channel();
        *self.rx.lock().unwrap() = Some(rx);
        (self.job_id, tx)
    }

    pub fn list_models(&mut self) {
        if self.running {
            return;
        }
        let base = match normalize_base_url(&self.base_url) {
            Ok(u) => u,
            Err(e) => {
                let (job, tx) = self.begin_job();
                let _ = tx.send(ServiceMsg::ListDone {
                    job,
                    models: Vec::new(),
                    log: format!("{e}\n"),
                });
                return;
            }
        };
        let headers = match self.auth_headers() {
            Ok(h) => h,
            Err(e) => {
                let (job, tx) = self.begin_job();
                let _ = tx.send(ServiceMsg::ListDone {
                    job,
                    models: Vec::new(),
                    log: format!("{e}\n"),
                });
                return;
            }
        };
        let client = self.client();
        let (job, tx) = self.begin_job();
        self.rt.spawn(async move {
            let url = format!("{base}/models");
            let mut log = format!("GET {url}\n");
            let models = match client.get(&url).headers(headers).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    log.push_str(&format!("HTTP {status}\n"));
                    if status.is_success() {
                        match parse_models_json(&text) {
                            Ok(ids) => {
                                log.push_str(&format!("发现 {} 个模型\n", ids.len()));
                                ids.into_iter()
                                    .map(|id| ModelRecord {
                                        id,
                                        selected: false,
                                        status: TestStatus::Pending,
                                        latency_ms: None,
                                        detail: String::new(),
                                    })
                                    .collect()
                            }
                            Err(e) => {
                                log.push_str(&format!(
                                    "{e}\n响应体: {}\n",
                                    sanitize(&truncate(&text, 800))
                                ));
                                Vec::new()
                            }
                        }
                    } else {
                        log.push_str(&format!("响应体: {}\n", sanitize(&truncate(&text, 800))));
                        Vec::new()
                    }
                }
                Err(e) => {
                    log.push_str(&format!("请求失败: {}\n", sanitize(&e.to_string())));
                    Vec::new()
                }
            };
            let _ = tx.send(ServiceMsg::ListDone { job, models, log });
        });
    }

    pub fn test_models(&mut self, models: &[ModelRecord]) {
        if self.running {
            return;
        }
        let ids = ids_to_test(models);
        if ids.is_empty() {
            let (job, tx) = self.begin_job();
            let _ = tx.send(ServiceMsg::TestDone {
                job,
                models: Vec::new(),
                log: "没有可验证的模型，请先获取模型列表\n".into(),
            });
            return;
        }
        let base = match normalize_base_url(&self.base_url) {
            Ok(u) => u,
            Err(e) => {
                let (job, tx) = self.begin_job();
                let _ = tx.send(ServiceMsg::TestDone {
                    job,
                    models: Vec::new(),
                    log: format!("{e}\n"),
                });
                return;
            }
        };
        let headers = match self.auth_headers() {
            Ok(h) => h,
            Err(e) => {
                let (job, tx) = self.begin_job();
                let _ = tx.send(ServiceMsg::TestDone {
                    job,
                    models: Vec::new(),
                    log: format!("{e}\n"),
                });
                return;
            }
        };
        let client = self.client();
        let (job, tx) = self.begin_job();
        self.rt.spawn(async move {
            run_tests(job, base, client, headers, ids, tx).await;
        });
    }
}

async fn run_tests(
    job: u64,
    base: String,
    client: reqwest::Client,
    headers: HeaderMap,
    ids: Vec<String>,
    tx: mpsc::Sender<ServiceMsg>,
) {
    let total = ids.len();
    let results = Arc::new(Mutex::new(
        ids.iter()
            .map(|id| ModelRecord {
                id: id.clone(),
                selected: true,
                status: TestStatus::Pending,
                latency_ms: None,
                detail: String::new(),
            })
            .collect::<Vec<_>>(),
    ));
    let url = format!("{base}/chat/completions");
    let sem = Arc::new(tokio::sync::Semaphore::new(4));
    let mut handles = Vec::with_capacity(total);

    for i in 0..total {
        let permit = match sem.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break,
        };
        let client = client.clone();
        let headers = headers.clone();
        let url = url.clone();
        let results = results.clone();
        let tx = tx.clone();
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            {
                let mut g = results.lock().unwrap();
                g[i].status = TestStatus::Testing;
            }
            let _ = tx.send(ServiceMsg::TestProgress {
                job,
                models: results.lock().unwrap().clone(),
            });

            let model_id = results.lock().unwrap()[i].id.clone();
            let body = serde_json::json!({
                "model": model_id,
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 1,
                "stream": false,
            });
            let start = Instant::now();
            let outcome = match client.post(&url).headers(headers).json(&body).send().await {
                Ok(resp) => {
                    let elapsed = start.elapsed().as_millis();
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    if status.is_success() {
                        (
                            TestStatus::Pass,
                            Some(elapsed),
                            format!("HTTP {status} 可用"),
                        )
                    } else {
                        (
                            TestStatus::Fail,
                            Some(elapsed),
                            format!(
                                "HTTP {status} {}",
                                sanitize(&truncate(&text, 200))
                            ),
                        )
                    }
                }
                Err(e) => (
                    TestStatus::Fail,
                    None,
                    format!("连接失败: {}", sanitize(&e.to_string())),
                ),
            };
            {
                let mut g = results.lock().unwrap();
                g[i].status = outcome.0;
                g[i].latency_ms = outcome.1;
                g[i].detail = outcome.2;
            }
            let _ = tx.send(ServiceMsg::TestProgress {
                job,
                models: results.lock().unwrap().clone(),
            });
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    let models = results.lock().unwrap().clone();
    let pass = models
        .iter()
        .filter(|r| r.status == TestStatus::Pass)
        .count();
    let mut log = String::new();
    for rec in &models {
        log.push_str(&format!("{} -> {}\n", rec.id, rec.detail));
    }
    log.push_str(&format!("验证完成: {pass}/{total} 个模型可用\n"));
    let _ = tx.send(ServiceMsg::TestDone { job, models, log });
}

pub fn normalize_base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("请填写请求地址".into());
    }
    let mut s = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    if let Some(i) = s.find(['?', '#']) {
        s.truncate(i);
    }
    loop {
        let next = s.trim_end_matches('/').to_string();
        let lower = next.to_ascii_lowercase();
        let cut = if lower.ends_with("/chat/completions") {
            next.len() - "/chat/completions".len()
        } else if lower.ends_with("/completions") {
            next.len() - "/completions".len()
        } else if lower.ends_with("/models") {
            next.len() - "/models".len()
        } else {
            s = next;
            break;
        };
        s = next[..cut].to_string();
    }
    if s == "http://" || s == "https://" {
        return Err("请求地址不完整".into());
    }
    Ok(s)
}

pub fn parse_models_json(text: &str) -> Result<Vec<String>, String> {
    let v: Value =
        serde_json::from_str(text).map_err(|_| "响应不是合法 JSON".to_string())?;
    let mut ids = Vec::new();
    let mut push = |item: &Value| {
        if let Some(s) = item.as_str() {
            let s = s.trim();
            if !s.is_empty() {
                ids.push(s.to_string());
            }
        } else if let Some(id) = item.get("id").and_then(|x| x.as_str()) {
            let id = id.trim();
            if !id.is_empty() {
                ids.push(id.to_string());
            }
        }
    };
    if let Some(arr) = v.as_array() {
        for item in arr {
            push(item);
        }
    } else if let Some(arr) = v.get("data").and_then(|x| x.as_array()) {
        for item in arr {
            push(item);
        }
    } else if let Some(arr) = v.get("models").and_then(|x| x.as_array()) {
        for item in arr {
            push(item);
        }
    } else {
        return Err("响应里没有模型列表（需要 data / models / 数组）".into());
    }
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Err("模型列表为空".into());
    }
    Ok(ids)
}

pub fn ids_to_test(models: &[ModelRecord]) -> Vec<String> {
    let selected: Vec<String> = models
        .iter()
        .filter(|m| m.selected)
        .map(|m| m.id.clone())
        .collect();
    if selected.is_empty() {
        models.iter().map(|m| m.id.clone()).collect()
    } else {
        selected
    }
}

fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() && !c.is_whitespace() {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut out = text.chars().take(max_chars).collect::<String>();
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_known_tails() {
        assert_eq!(
            normalize_base_url("https://api.example.com/v1/models/").unwrap(),
            "https://api.example.com/v1"
        );
        assert_eq!(
            normalize_base_url("api.example.com/v1/chat/completions").unwrap(),
            "https://api.example.com/v1"
        );
        assert_eq!(
            normalize_base_url("  https://api.example.com/v1  ").unwrap(),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn normalize_rejects_empty() {
        assert!(normalize_base_url("   ").is_err());
    }

    #[test]
    fn parse_openai_and_array() {
        let openai = r#"{"object":"list","data":[{"id":"gpt-4"},{"id":"gpt-4"}]}"#;
        assert_eq!(parse_models_json(openai).unwrap(), vec!["gpt-4"]);
        let arr = r#"["b","a"]"#;
        assert_eq!(parse_models_json(arr).unwrap(), vec!["a", "b"]);
        let models = r#"{"models":[{"id":"claude"}]}"#;
        assert_eq!(parse_models_json(models).unwrap(), vec!["claude"]);
    }

    #[test]
    fn ids_to_test_selected_or_all() {
        let mk = |id: &str, selected: bool| ModelRecord {
            id: id.into(),
            selected,
            status: TestStatus::Pending,
            latency_ms: None,
            detail: String::new(),
        };
        let all = vec![mk("a", false), mk("b", false)];
        assert_eq!(ids_to_test(&all), vec!["a", "b"]);
        let some = vec![mk("a", false), mk("b", true)];
        assert_eq!(ids_to_test(&some), vec!["b"]);
    }
}
