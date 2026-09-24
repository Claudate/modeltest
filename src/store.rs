//! 本机持久化的只有「请求地址 + Token」这一组连接配置，不存模型清单。
//!
//! 文件在 Application Support/model-test/endpoints.json，写入时权限收到 0600
//! （Token 是密钥，只有当前用户可读）。同一 base_url 覆盖更新并移到最前，
//! 下次启动直接回填第一条，用户不必重复粘贴。测试用 `MODEL_TEST_DATA_DIR` 改路径。

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedEndpoint {
    /// 规范化后的 Base URL（已剥掉 /models、/chat/completions 尾巴）。
    pub base_url: String,
    /// 原样保存，界面上只显示掩码。
    pub token: String,
    pub saved_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EndpointStore {
    pub entries: Vec<SavedEndpoint>,
}

impl EndpointStore {
    pub fn load() -> Self {
        Self::load_from(&data_path())
    }

    pub fn load_from(path: &Path) -> Self {
        let Ok(raw) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&data_path())
    }

    /// 先写内容再把权限压到 0600：文件可能已存在且权限更宽，不能只靠 create 时的 mode。
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("无法创建保存目录: {e}"))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(path).map_err(|e| format!("写入失败: {e}"))?;
        file.write_all(json.as_bytes())
            .map_err(|e| format!("写入失败: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// 完全相同的地址 + Token 已在最前时不用再写一遍（避免每轮验证都刷时间戳）。
    pub fn is_current(&self, base_url: &str, token: &str) -> bool {
        self.entries
            .first()
            .is_some_and(|e| e.base_url == base_url && e.token == token)
    }

    /// 已存在（同 base_url）则覆盖并移到最前。
    pub fn upsert(&mut self, entry: SavedEndpoint) {
        self.entries
            .retain(|e| e.base_url != entry.base_url);
        self.entries.insert(0, entry);
    }

    pub fn remove_at(&mut self, index: usize) {
        if index < self.entries.len() {
            self.entries.remove(index);
        }
    }

    /// 按地址搜索；Token 不参与匹配，也不出现在结果文本里。
    pub fn matching(&self, query: &str) -> Vec<usize> {
        let q = query.trim().to_ascii_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| q.is_empty() || e.base_url.to_ascii_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect()
    }
}

/// Token 在界面上只显示掩码：保留头 3 尾 4，其余打点。
pub fn mask_token(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 8 {
        return "•".repeat(chars.len().max(4));
    }
    let head: String = chars[..3].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

pub fn data_path() -> PathBuf {
    if let Ok(dir) = std::env::var("MODEL_TEST_DATA_DIR") {
        return PathBuf::from(dir).join("endpoints.json");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join("Library/Application Support/model-test/endpoints.json")
}

pub fn now_iso_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    civil_utc(secs)
}

/// Howard Hinnant civil_from_days：不引 chrono，测试也能钉死格式。
pub fn civil_utc(secs: u64) -> String {
    let z = secs as i64;
    let days = z.div_euclid(86400);
    let tod = z.rem_euclid(86400) as u64;
    let h = tod / 3600;
    let m = (tod % 3600) / 60;
    let s = tod % 60;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    format!("{y:04}-{mth:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// 落盘时间戳是 UTC（文件跟着机器走时不该带本地时区），界面上要显示成本机时间。
/// stable 的 std 拿不到本地偏移（`SystemTimeExt::local_offset` 还在 nightly），
/// 又不想为这一件事引 chrono：启动后跑一次 `/bin/date +%z` 取偏移并缓存，
/// 取不到就退回 0（显示成 UTC），界面不会因为时区而报错。
pub fn local_offset_secs() -> i64 {
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        let parsed = std::process::Command::new("/bin/date")
            .arg("+%z")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| parse_zone_offset(s.trim()));
        parsed.unwrap_or(0)
    })
}

/// 解析 `+0800` / `-0530` 这类 ISO 8601 时区后缀为秒；认不了返回 None。
fn parse_zone_offset(s: &str) -> Option<i64> {
    let sign = match s.as_bytes().first()? {
        b'+' => 1i64,
        b'-' => -1i64,
        _ => return None,
    };
    let digits: String = s[1..].chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 2 {
        return None;
    }
    let hours: i64 = digits[..2].parse().ok()?;
    let minutes: i64 = if digits.len() >= 4 {
        digits[2..4].parse().ok()?
    } else {
        0
    };
    if hours > 14 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3600 + minutes * 60))
}

/// `civil_utc` 的反向：把 `2026-09-08T11:55:58Z` 解回 unix 秒。
fn parse_utc_secs(iso: &str) -> Option<i64> {
    let (date, time) = iso.split_once('T')?;
    let mut d = date.split('-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    let mut t = time.trim_end_matches('Z').split(':');
    let hour: i64 = t.next()?.parse().ok()?;
    let min: i64 = t.next()?.parse().ok()?;
    let sec: i64 = t.next().unwrap_or("0").parse().unwrap_or(0);
    if !(0..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Howard Hinnant days_from_civil
    let y = year - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    // 上面整段算出来的是「距 1970-01-01 的天数」，必须乘 86400 才是秒。
    let days = era * 146097 + doe - 719468;
    Some(days * 86_400 + hour * 3600 + min * 60 + sec)
}

/// 显示用的 `2026-09-08 19:55`（本机时间，精确到分钟）。
/// 解析不了的历史数据原样回显，宁可显示得丑也不要显示成空白。
pub fn display_time(iso_utc: &str) -> String {
    display_time_with_offset(iso_utc, local_offset_secs())
}

fn display_time_with_offset(iso_utc: &str, offset_secs: i64) -> String {
    let Some(secs) = parse_utc_secs(iso_utc) else {
        return iso_utc.to_owned();
    };
    match secs.checked_add(offset_secs) {
        Some(local) if local >= 0 => {
            let head: String = civil_utc(local as u64).chars().take(16).collect();
            head.replace('T', " ")
        }
        _ => iso_utc.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn tmp_path() -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("model-test-store-{n}.json"))
    }

    #[test]
    fn civil_unix_epoch() {
        assert_eq!(civil_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(civil_utc(1_704_067_200), "2024-01-01T00:00:00Z");
    }

    #[test]
    fn struct_has_no_model_field() {
        // 持久化对象只有地址 / Token / 时间：模型清单不落盘。
        let json = serde_json::to_string(&SavedEndpoint {
            base_url: "https://gw.example.com/v1".into(),
            token: "sk-abcdef123456".into(),
            saved_at: "2026-09-08T00:00:00Z".into(),
        })
        .unwrap();
        assert!(!json.contains("model"));
        assert!(json.contains("base_url"));
        assert!(json.contains("token"));
    }

    #[test]
    fn upsert_dedups_by_base_url_and_saves() {
        let path = tmp_path();
        let _ = fs::remove_file(&path);
        let mut store = EndpointStore::default();
        store.upsert(SavedEndpoint {
            base_url: "https://a.example.com/v1".into(),
            token: "sk-old".into(),
            saved_at: "2026-09-08T00:00:00Z".into(),
        });
        store.upsert(SavedEndpoint {
            base_url: "https://a.example.com/v1".into(),
            token: "sk-new".into(),
            saved_at: "2026-09-08T01:00:00Z".into(),
        });
        assert_eq!(store.entries.len(), 1);
        assert_eq!(store.entries[0].token, "sk-new");
        assert!(store.is_current("https://a.example.com/v1", "sk-new"));
        assert!(!store.is_current("https://a.example.com/v1", "sk-old"));
        store.save_to(&path).unwrap();
        let loaded = EndpointStore::load_from(&path);
        assert_eq!(loaded.entries[0].base_url, "https://a.example.com/v1");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "Token 文件必须只有本人可读");
        }
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn matching_searches_url_only_and_mask_hides_middle() {
        let mut store = EndpointStore::default();
        store.upsert(SavedEndpoint {
            base_url: "https://newapi.picpop.com.cn/v1".into(),
            token: "sk-1234567890abcdef".into(),
            saved_at: "t".into(),
        });
        assert_eq!(store.matching("PICPOP").len(), 1);
        assert_eq!(store.matching("sk-1234567890").len(), 0);
        assert!(store.matching("openai").is_empty());
        assert_eq!(mask_token("sk-1234567890abcdef"), "sk-…cdef");
        // 短 Token 不暴露任何字符：整串打点，长度不足 4 时补足到 4 位。
        assert_eq!(mask_token("short"), "•••••");
        assert_eq!(mask_token("ab"), "••••");
    }

    #[test]
    fn zone_offset_parses_date_style_suffixes() {
        assert_eq!(parse_zone_offset("+0800"), Some(8 * 3600));
        assert_eq!(parse_zone_offset("-0530"), Some(-(5 * 3600 + 30 * 60)));
        assert_eq!(parse_zone_offset("+0000"), Some(0));
        assert_eq!(parse_zone_offset("+08"), Some(8 * 3600));
        assert_eq!(parse_zone_offset("UTC"), None);
        assert_eq!(parse_zone_offset("+9900"), None);
    }

    #[test]
    fn utc_secs_roundtrips_civil_utc() {
        for iso in [
            "1970-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
            "2026-09-08T11:55:58Z",
            // 闰日要用真的闰年：2026 不是闰年，写成 02-29 会滚到 03-01。
            "2024-02-29T00:00:00Z",
        ] {
            let secs = parse_utc_secs(iso).expect("应能解析");
            assert_eq!(civil_utc(secs as u64), iso, "{iso} 往返不一致");
        }
        assert_eq!(parse_utc_secs("not-a-date"), None);
    }

    #[test]
    fn display_time_shifts_to_local_and_falls_back() {
        // UTC 11:55 + 8h = 本机 19:55，且只留到分钟。
        assert_eq!(
            display_time_with_offset("2026-09-08T11:55:58Z", 8 * 3600),
            "2026-09-08 19:55"
        );
        // 跨日回退：UTC 11:55 在 -05:00 是当天 06:55。
        assert_eq!(
            display_time_with_offset("2026-09-08T11:55:58Z", -5 * 3600),
            "2026-09-08 06:55"
        );
        // 跨日到前一天。
        assert_eq!(
            display_time_with_offset("2026-09-08T01:00:00Z", -5 * 3600),
            "2026-09-07 20:00"
        );
        // 偏移取不到（退回 0）时显示 UTC 时刻，不显示空白。
        assert_eq!(
            display_time_with_offset("2026-09-08T11:55:58Z", 0),
            "2026-09-08 11:55"
        );
        // 手改过的脏数据原样回显。
        assert_eq!(display_time_with_offset("t", 8 * 3600), "t");
    }
}
