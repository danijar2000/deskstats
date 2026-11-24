use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub struct Settings {
    path: String,
    pub interval_ms: AtomicU64,
    pub lan: AtomicBool,
    pub alerts: AtomicBool,
    pub weather_min: AtomicU64,
}

const INTERVAL: (u64, u64) = (200, 60_000);
const WEATHER: (u64, u64) = (15, 10_080);

impl Settings {
    pub fn load(path: String) -> Self {
        let s = Settings {
            path,
            interval_ms: AtomicU64::new(1000),
            lan: AtomicBool::new(false),
            alerts: AtomicBool::new(true),
            weather_min: AtomicU64::new(60),
        };
        if let Ok(text) = std::fs::read_to_string(&s.path) {
            s.apply(&text.lines().collect::<Vec<_>>().join("&"));
        }
        s
    }

    pub fn apply(&self, query: &str) {
        for (key, value) in query.split('&').filter_map(|p| p.split_once('=')) {
            let number = value.parse::<u64>().ok();
            let flag = match value {
                "1" | "true" => Some(true),
                "0" | "false" => Some(false),
                _ => None,
            };
            match key {
                "interval_ms" => {
                    if let Some(v) = number {
                        self.interval_ms.store(v.clamp(INTERVAL.0, INTERVAL.1), Ordering::Relaxed);
                    }
                }
                "weather_min" => {
                    if let Some(v) = number {
                        self.weather_min.store(v.clamp(WEATHER.0, WEATHER.1), Ordering::Relaxed);
                    }
                }
                "lan" => {
                    if let Some(v) = flag {
                        self.lan.store(v, Ordering::Relaxed);
                    }
                }
                "alerts" => {
                    if let Some(v) = flag {
                        self.alerts.store(v, Ordering::Relaxed);
                    }
                }
                _ => {}
            }
        }
    }

    fn lines(&self) -> String {
        format!(
            "interval_ms={}\nlan={}\nalerts={}\nweather_min={}\n",
            self.interval_ms.load(Ordering::Relaxed),
            self.lan.load(Ordering::Relaxed) as u8,
            self.alerts.load(Ordering::Relaxed) as u8,
            self.weather_min.load(Ordering::Relaxed),
        )
    }

    pub fn save(&self) {
        let temp = format!("{}.new", self.path);
        if std::fs::write(&temp, self.lines()).is_ok() {
            let _ = std::fs::rename(&temp, &self.path);
        }
    }

    pub fn json(&self) -> String {
        format!(
            "{{\"interval_ms\":{},\"lan\":{},\"alerts\":{},\"weather_min\":{}}}",
            self.interval_ms.load(Ordering::Relaxed),
            self.lan.load(Ordering::Relaxed),
            self.alerts.load(Ordering::Relaxed),
            self.weather_min.load(Ordering::Relaxed),
        )
    }

    #[cfg(test)]
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn interval(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.interval_ms.load(Ordering::Relaxed))
    }

    pub fn idle_after(&self) -> std::time::Duration {
        (self.interval() * 3).max(std::time::Duration::from_secs(5))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_clamped_kept_and_stored() {
        let path = std::env::temp_dir().join(format!("deskstats-settings-{}", std::process::id()));
        let path = path.to_string_lossy().into_owned();
        let _ = std::fs::remove_file(&path);
        let s = Settings::load(path.clone());
        assert_eq!(s.json(), r#"{"interval_ms":1000,"lan":false,"alerts":true,"weather_min":60}"#);
        s.apply("interval_ms=5&lan=1&alerts=x&weather_min=99999&other=1");
        assert_eq!(s.json(), r#"{"interval_ms":200,"lan":true,"alerts":true,"weather_min":10080}"#);
        s.save();
        let again = Settings::load(path.clone());
        assert_eq!(again.json(), s.json());
        std::fs::remove_file(&path).unwrap();
    }
}
