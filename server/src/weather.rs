//! Current weather from open-meteo.com (no key needed; only a city name or coordinates leave this Mac).
//!
//! Two kinds of places: the city given at startup (`--city`), and places the phone picks from
//! its own list and asks for by coordinates (`/stats?place=<lat>,<lon>`). A place the phone no
//! longer asks for is forgotten.
//!
//! All times here are wall-clock milliseconds: the monotonic clock stands still while the Mac
//! sleeps, and yesterday's weather would pass for fresh in the morning.

use crate::util::{json_escape, log, now_ms};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How many places picked on the phone are kept at once.
const MAX_PLACES: usize = 4;

/// The last good weather as a JSON object and when it was fetched (ms since the epoch).
type Report = Option<(String, u64)>;

/// Milliseconds since `then`; zero when the clock was set back.
fn since(then: u64) -> u64 {
    now_ms().saturating_sub(then)
}

struct Place {
    /// Coordinates in hundredths of a degree.
    lat: i32,
    lon: i32,
    report: Report,
    tried: Option<u64>,
    asked: u64,
}

#[derive(Default)]
pub struct Store {
    city: Report,
    places: Vec<Place>,
}

pub struct Weather {
    store: Mutex<Store>,
    every: u64,
    /// Weather older than two periods (plus a margin for a slow network) is not shown.
    max_age: u64,
    worker: Mutex<Option<std::thread::Thread>>,
}

/// "42.87,74.59" as hundredths of a degree.
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
        let every = every.as_millis() as u64;
        Weather { store: Mutex::new(Store::default()), every, max_age: every * 2 + 1_800_000, worker: Mutex::new(None) }
    }

    fn fresh(&self, report: &Report) -> String {
        match report {
            Some((json, fetched)) if since(*fetched) <= self.max_age => json.clone(),
            _ => String::from("null"),
        }
    }

    /// The weather of the startup city as JSON, or "null".
    pub fn city(&self) -> String {
        self.fresh(&self.store.lock().unwrap_or_else(|e| e.into_inner()).city)
    }

    /// Sets the startup city's weather, for tests of the HTTP side.
    #[cfg(test)]
    pub fn set_city(&self, json: &str) {
        self.store.lock().unwrap_or_else(|e| e.into_inner()).city = Some((json.to_string(), now_ms()));
    }

    /// The weather of a place the phone picked, or "null" until it is fetched. Asking keeps the
    /// place alive; a new one wakes the worker, so the answer is there a few seconds later.
    pub fn place(&self, lat: i32, lon: i32) -> String {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = store.places.iter_mut().find(|p| p.lat == lat && p.lon == lon) {
            p.asked = now_ms();
            return self.fresh(&p.report);
        }
        if store.places.len() >= MAX_PLACES {
            if let Some(oldest) = (0..store.places.len()).min_by_key(|i| store.places[*i].asked) {
                store.places.swap_remove(oldest);
            }
        }
        store.places.push(Place { lat, lon, report: None, tried: None, asked: now_ms() });
        drop(store);
        if let Some(worker) = &*self.worker.lock().unwrap_or_else(|e| e.into_inner()) {
            worker.unpark();
        }
        String::from("null")
    }
}

/// `-q` first: curl must not read ~/.curlrc, which any program of the user can write, while it
/// runs with this server's right to read the whole disk.
fn curl(url: &str) -> Option<String> {
    let (ok, text) = crate::util::run("/usr/bin/curl", &["-q", "-fsS", "--max-time", "20", "--max-filesize", "200000", "--proto", "=https", url], Duration::from_secs(25))?;
    ok.then_some(text)
}

fn url_encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The number that follows `"key":` (or opens the array there) in a JSON text; None for null.
pub fn number_after(json: &str, key: &str) -> Option<f64> {
    let marker = format!("\"{key}\":");
    let rest = json[json.find(&marker)? + marker.len()..].trim_start_matches(['[', ' ']);
    let end = rest.find(|c: char| !(c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Coordinates and the country of the first place the geocoder finds for `city`.
fn geocode(city: &str) -> Option<(f64, f64, String)> {
    let json = curl(&format!("https://geocoding-api.open-meteo.com/v1/search?count=1&format=json&name={}", url_encode(city)))?;
    let lat = number_after(&json, "latitude")?;
    let lon = number_after(&json, "longitude")?;
    let country = json
        .find("\"country\":\"")
        .map(|i| &json[i + 11..])
        .and_then(|r| r.find('"').map(|e| r[..e].to_string()))
        .unwrap_or_default();
    Some((lat, lon, country))
}

fn field(value: Option<f64>, digits: usize) -> String {
    match value {
        Some(v) if v.is_finite() => format!("{v:.digits$}"),
        _ => String::from("null"),
    }
}

/// Current weather as a JSON object. Fields the API leaves empty are null; the temperature is required.
fn weather_json(city: &str, lat: f64, lon: f64) -> Option<String> {
    let json = curl(&format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&timezone=auto&forecast_days=1\
         &current=temperature_2m,apparent_temperature,weather_code,is_day&daily=temperature_2m_max,temperature_2m_min"
    ))?;
    // "current_units" and "daily_units" carry the same keys with unit strings, so start at the data blocks.
    let current = &json[json.find("\"current\":{")?..];
    let daily = json.find("\"daily\":{").map(|i| &json[i..]).unwrap_or("");
    let temp = number_after(current, "temperature_2m")?;
    Some(format!(
        "{{\"city\":\"{}\",\"temp\":{},\"feels\":{},\"code\":{},\"is_day\":{},\"max\":{},\"min\":{}}}",
        json_escape(city),
        field(Some(temp), 1),
        field(number_after(current, "apparent_temperature"), 1),
        field(number_after(current, "weather_code"), 0),
        field(number_after(current, "is_day"), 0),
        field(number_after(daily, "temperature_2m_max"), 1),
        field(number_after(daily, "temperature_2m_min"), 1),
    ))
}

/// Keeps the weather current: the startup city every `every` (a failure is retried after 5
/// minutes; the place is looked up again once a day, so a wrong first match does not stick for
/// good), and each place the phone asks for. Between fetches the thread is parked.
pub fn run(city: Option<String>, weather: Arc<Weather>) {
    *weather.worker.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::thread::current());
    let every = weather.every;
    let retry = 300_000.min(every);
    let mut located: Option<(f64, f64, u64)> = None;
    let mut city_due = 0u64;
    let mut failing = false;
    loop {
        if let Some(city) = city.as_ref().filter(|_| now_ms() >= city_due) {
            if located.is_none_or(|(_, _, at)| since(at) >= 24 * 3_600_000) {
                match geocode(city) {
                    Some((lat, lon, country)) => {
                        if located.is_none() {
                            log(&format!("weather: \"{city}\" is {lat:.3}, {lon:.3} ({country})"));
                        }
                        located = Some((lat, lon, now_ms()));
                    }
                    None if located.is_none() => log(&format!("weather: cannot find \"{city}\"")),
                    None => {}
                }
            }
            let fresh = located.and_then(|(lat, lon, _)| weather_json(city, lat, lon));
            city_due = now_ms() + if fresh.is_some() { every } else { retry };
            match fresh {
                Some(json) => {
                    weather.store.lock().unwrap_or_else(|e| e.into_inner()).city = Some((json, now_ms()));
                    if failing {
                        log("weather: back");
                    }
                    failing = false;
                }
                None => {
                    if !failing {
                        log("weather: request failed, retrying in 5 minutes");
                    }
                    failing = true;
                }
            }
        }

        // Places picked on the phone: forget those it stopped asking for, fetch those that are due.
        let due: Vec<(i32, i32)> = {
            let mut store = weather.store.lock().unwrap_or_else(|e| e.into_inner());
            store.places.retain(|p| since(p.asked) < every * 2);
            let wait = |p: &Place| if p.report.as_ref().is_some_and(|(_, at)| since(*at) < every) { every } else { retry };
            store.places.iter().filter(|p| p.tried.is_none_or(|t| since(t) >= wait(p))).map(|p| (p.lat, p.lon)).collect()
        };
        for (lat, lon) in due {
            let fresh = weather_json("", lat as f64 / 100.0, lon as f64 / 100.0);
            let mut store = weather.store.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(p) = store.places.iter_mut().find(|p| p.lat == lat && p.lon == lon) {
                p.tried = Some(now_ms());
                if let Some(json) = fresh {
                    p.report = Some((json, now_ms()));
                }
            }
        }

        // Sleep until the next fetch is due; a newly asked place unparks the thread earlier. A
        // park is at most a minute long, because its timer does not run while the Mac sleeps.
        let next = {
            let store = weather.store.lock().unwrap_or_else(|e| e.into_inner());
            let untried = store.places.iter().any(|p| p.tried.is_none());
            let places = if store.places.is_empty() { every } else { retry };
            if untried {
                0
            } else if city.is_some() {
                city_due.saturating_sub(now_ms()).min(places)
            } else {
                places
            }
        };
        if next > 0 {
            std::thread::park_timeout(Duration::from_millis(next.clamp(1000, 60_000)));
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
    fn url_encode_escapes_everything_but_alphanumerics() {
        assert_eq!(url_encode("New York"), "New%20York");
        assert_eq!(url_encode("Бишкек"), "%D0%91%D0%B8%D1%88%D0%BA%D0%B5%D0%BA");
    }

    #[test]
    fn places_are_parsed_kept_and_capped() {
        assert_eq!(parse_place("42.87,74.59"), Some((4287, 7459)));
        assert_eq!(parse_place("-33.87,151.21"), Some((-3387, 15121)));
        assert_eq!(parse_place("91,0"), None);
        assert_eq!(parse_place("Bishkek"), None);
        let w = Weather::new(Duration::from_secs(3600));
        assert_eq!(w.city(), "null");
        for i in 0..6 {
            assert_eq!(w.place(i, i), "null");
        }
        let mut store = w.store.lock().unwrap();
        assert_eq!(store.places.len(), MAX_PLACES);
        store.places[0].report = Some(("{}".into(), now_ms()));
        let (lat, lon) = (store.places[0].lat, store.places[0].lon);
        drop(store);
        assert_eq!(w.place(lat, lon), "{}");
    }

    #[test]
    fn old_weather_is_not_served() {
        let w = Weather::new(Duration::from_secs(3600));
        w.store.lock().unwrap().city = Some(("{}".into(), now_ms() - 3 * 3_600_000));
        assert_eq!(w.city(), "null");
        w.store.lock().unwrap().city = Some(("{}".into(), now_ms() - 3_600_000));
        assert_eq!(w.city(), "{}");
    }
}
