use crate::util::now_ms;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_PLACES: usize = 4;
const RETRY_MS: u64 = 300_000;

type Report = Option<(String, u64)>;

fn since(then: u64) -> u64 {
    now_ms().saturating_sub(then)
}

struct Place {
    lat: i32,
    lon: i32,
    report: Report,
    tried: Option<u64>,
    asked: u64,
}

pub struct Weather {
    places: Mutex<Vec<Place>>,
    every_ms: AtomicU64,
    worker: Mutex<Option<std::thread::Thread>>,
}

pub fn parse_place(text: &str) -> Option<(i32, i32)> {
    let (lat, lon) = text.split_once(',')?;
    let (lat, lon): (f64, f64) = (lat.parse().ok()?, lon.parse().ok()?);
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some(((lat * 100.0).round() as i32, (lon * 100.0).round() as i32))
}

impl Weather {
    pub fn new(every: Duration) -> Self {
        Weather { places: Mutex::new(Vec::new()), every_ms: AtomicU64::new(every.as_millis() as u64), worker: Mutex::new(None) }
    }

    pub fn set_every(&self, every: Duration) {
        self.every_ms.store(every.as_millis() as u64, Ordering::Relaxed);
        self.wake();
    }

    fn every(&self) -> u64 {
        self.every_ms.load(Ordering::Relaxed).max(60_000)
    }

    fn wake(&self) {
        if let Some(worker) = &*self.worker.lock().unwrap_or_else(|e| e.into_inner()) {
            worker.unpark();
        }
    }

    pub fn place(&self, lat: i32, lon: i32) -> String {
        let max_age = self.every() * 2 + 1_800_000;
        let mut places = self.places.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = places.iter_mut().find(|p| p.lat == lat && p.lon == lon) {
            p.asked = now_ms();
            return match &p.report {
                Some((json, fetched)) if since(*fetched) <= max_age => json.clone(),
                _ => String::from("null"),
            };
        }
        if places.len() >= MAX_PLACES {
            if let Some(oldest) = (0..places.len()).min_by_key(|i| places[*i].asked) {
                places.swap_remove(oldest);
            }
        }
        places.push(Place { lat, lon, report: None, tried: None, asked: now_ms() });
        drop(places);
        self.wake();
        String::from("null")
    }

    #[cfg(test)]
    fn set_report(&self, lat: i32, lon: i32, json: &str, fetched: u64) {
        let mut places = self.places.lock().unwrap();
        if let Some(p) = places.iter_mut().find(|p| p.lat == lat && p.lon == lon) {
            p.report = Some((json.to_string(), fetched));
        }
    }
}

// -q: curl must not read ~/.curlrc while it runs with this server's disk access.
fn curl(url: &str) -> Option<String> {
    let (ok, text) = crate::util::run(crate::sys::CURL, &["-q", "-fsS", "--max-time", "20", "--max-filesize", "200000", "--proto", "=https", url], Duration::from_secs(25))?;
    ok.then_some(text)
}

pub fn number_after(json: &str, key: &str) -> Option<f64> {
    let marker = format!("\"{key}\":");
    let rest = json[json.find(&marker)? + marker.len()..].trim_start_matches(['[', ' ']);
    let end = rest.find(|c: char| !(c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn field(value: Option<f64>, digits: usize) -> String {
    match value {
        Some(v) if v.is_finite() => format!("{v:.digits$}"),
        _ => String::from("null"),
    }
}

fn weather_json(lat: f64, lon: f64) -> Option<String> {
    let json = curl(&format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&timezone=auto&forecast_days=1\
         &current=temperature_2m,apparent_temperature,weather_code,is_day&daily=temperature_2m_max,temperature_2m_min,sunrise,sunset&timeformat=unixtime"
    ))?;
    let current = &json[json.find("\"current\":{")?..];
    let daily = json.find("\"daily\":{").map(|i| &json[i..]).unwrap_or("");
    let temp = number_after(current, "temperature_2m")?;
    Some(format!(
        "{{\"temp\":{},\"feels\":{},\"code\":{},\"is_day\":{},\"max\":{},\"min\":{},\"sunrise\":{},\"sunset\":{}}}",
        field(Some(temp), 1),
        field(number_after(current, "apparent_temperature"), 1),
        field(number_after(current, "weather_code"), 0),
        field(number_after(current, "is_day"), 0),
        field(number_after(daily, "temperature_2m_max"), 1),
        field(number_after(daily, "temperature_2m_min"), 1),
        field(number_after(daily, "sunrise"), 0),
        field(number_after(daily, "sunset"), 0),
    ))
}

pub fn run(weather: Arc<Weather>) {
    *weather.worker.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::thread::current());
    loop {
        let every = weather.every();
        let retry = RETRY_MS.min(every);
        let due: Vec<(i32, i32)> = {
            let mut places = weather.places.lock().unwrap_or_else(|e| e.into_inner());
            places.retain(|p| since(p.asked) < every * 2);
            let wait = |p: &Place| if p.report.as_ref().is_some_and(|(_, at)| since(*at) < every) { every } else { retry };
            places.iter().filter(|p| p.tried.is_none_or(|t| since(t) >= wait(p))).map(|p| (p.lat, p.lon)).collect()
        };
        for (lat, lon) in due {
            let fresh = weather_json(lat as f64 / 100.0, lon as f64 / 100.0);
            let mut places = weather.places.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(p) = places.iter_mut().find(|p| p.lat == lat && p.lon == lon) {
                p.tried = Some(now_ms());
                if let Some(json) = fresh {
                    p.report = Some((json, now_ms()));
                }
            }
        }
        let (none, untried) = {
            let places = weather.places.lock().unwrap_or_else(|e| e.into_inner());
            (places.is_empty(), places.iter().any(|p| p.tried.is_none()))
        };
        if none {
            // Nobody has asked for weather lately: sleep until a phone asks for a city.
            std::thread::park();
        } else if !untried {
            // Short parks: their timer stands still while the Mac sleeps.
            std::thread::park_timeout(Duration::from_millis(retry.clamp(1000, 60_000)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_after_reads_numbers_and_arrays() {
        let j = r#"{"temperature_2m":-3.5,"temperature_2m_max":[12.4],"is_day":0,"x":1e2,"n":null}"#;
        assert_eq!(number_after(j, "temperature_2m"), Some(-3.5));
        assert_eq!(number_after(j, "temperature_2m_max"), Some(12.4));
        assert_eq!(number_after(j, "is_day"), Some(0.0));
        assert_eq!(number_after(j, "x"), Some(100.0));
        assert_eq!(number_after(j, "n"), None);
        assert_eq!(number_after(j, "missing"), None);
    }

    #[test]
    fn places_are_parsed_kept_and_capped() {
        assert_eq!(parse_place("42.87,74.59"), Some((4287, 7459)));
        assert_eq!(parse_place("-33.87,151.21"), Some((-3387, 15121)));
        assert_eq!(parse_place("91,0"), None);
        assert_eq!(parse_place("Bishkek"), None);
        let w = Weather::new(Duration::from_secs(3600));
        for i in 0..6 {
            assert_eq!(w.place(i, i), "null");
        }
        assert_eq!(w.places.lock().unwrap().len(), MAX_PLACES);
        w.set_report(5, 5, "{}", now_ms());
        assert_eq!(w.place(5, 5), "{}");
    }

    #[test]
    fn old_weather_is_not_served() {
        let w = Weather::new(Duration::from_secs(3600));
        w.place(1, 1);
        w.set_report(1, 1, "{}", now_ms() - 3 * 3_600_000);
        assert_eq!(w.place(1, 1), "null");
        w.set_report(1, 1, "{}", now_ms() - 3_600_000);
        assert_eq!(w.place(1, 1), "{}");
    }
}
