use super::{Feed, FeedError, FetchOutput};
use crate::domain::entities::intel_entry::IntelEntry;
use crate::domain::values::category::Category;
use crate::domain::values::confidence::Confidence;
use crate::domain::values::source_type::SourceType;
use async_trait::async_trait;
use std::time::Duration;

/// NWS Weather forecast feed. Fetches forecast for a specific grid point.
/// Default: Central Park, NYC (OKX/33,37) — matches Kalshi KXHIGHNY resolution source.
pub struct NwsFeed {
    /// NWS grid office (e.g., "OKX")
    office: String,
    /// Grid X coordinate
    grid_x: u32,
    /// Grid Y coordinate
    grid_y: u32,
    /// Location label for tags/titles
    location: String,
    client: reqwest::Client,
}

impl NwsFeed {
    /// Create a feed for NWS Central Park (default for Kalshi NYC weather).
    pub fn central_park() -> Self {
        Self {
            office: "OKX".into(),
            grid_x: 33,
            grid_y: 37,
            location: "NYC".into(),
            client: reqwest::Client::builder()
                .user_agent("OpenIntel/0.1 (github.com/Kloudy-Sky/openintel)")
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Create a feed for a custom NWS grid point.
    pub fn new(office: String, grid_x: u32, grid_y: u32, location: String) -> Self {
        Self {
            office,
            grid_x,
            grid_y,
            location,
            client: reqwest::Client::builder()
                .user_agent("OpenIntel/0.1 (github.com/Kloudy-Sky/openintel)")
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct ForecastResponse {
    properties: ForecastProperties,
}

#[derive(Debug, serde::Deserialize)]
struct ForecastProperties {
    periods: Vec<ForecastPeriod>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForecastPeriod {
    name: String,
    temperature: i32,
    temperature_unit: String,
    short_forecast: String,
    is_daytime: bool,
    #[serde(default)]
    wind_speed: Option<String>,
    #[serde(default)]
    wind_direction: Option<String>,
}

#[async_trait]
impl Feed for NwsFeed {
    fn name(&self) -> &str {
        "nws_weather"
    }

    async fn fetch(&self) -> Result<FetchOutput, FeedError> {
        let url = format!(
            "https://api.weather.gov/gridpoints/{}/{},{}/forecast",
            self.office, self.grid_x, self.grid_y
        );

        let resp = self
            .client
            .get(&url)
            .header("Accept", "application/geo+json")
            .send()
            .await
            .map_err(|e| FeedError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(FeedError::Network(format!(
                "NWS API returned {}",
                resp.status()
            )));
        }

        let data: ForecastResponse = resp
            .json()
            .await
            .map_err(|e| FeedError::Parse(e.to_string()))?;

        // Take up to 6 periods (3 days of day/night)
        let entries: Vec<IntelEntry> = data
            .properties
            .periods
            .into_iter()
            .take(6)
            .filter(|p| p.is_daytime) // Only daytime highs for weather trading
            .map(|period| {
                let title = format!(
                    "NWS {} {} forecast: {}°{}",
                    self.location, period.name, period.temperature, period.temperature_unit
                );

                let mut body = format!(
                    "{}: High {}°{} — {}",
                    period.name, period.temperature, period.temperature_unit, period.short_forecast
                );

                if let (Some(speed), Some(dir)) = (&period.wind_speed, &period.wind_direction) {
                    body.push_str(&format!(". Wind: {dir} {speed}"));
                }

                let tags = vec![
                    "weather".to_string(),
                    "NWS".to_string(),
                    self.location.clone(),
                    "nws-feed".to_string(),
                    format!("{}{}", period.temperature, period.temperature_unit),
                    period.name.to_lowercase().replace(' ', "-"),
                ];

                // NWS forecasts for today are higher confidence than multi-day
                let conf =
                    if period.name.contains("Today") || period.name.contains("This Afternoon") {
                        0.85
                    } else {
                        0.7
                    };

                IntelEntry::new(
                    Category::Weather,
                    title,
                    body,
                    Some("nws".to_string()),
                    tags,
                    Confidence::new(conf).unwrap_or_else(|_| Confidence::new(0.5).unwrap()),
                    true, // Weather forecasts are always actionable for trading
                    SourceType::External,
                    Some(serde_json::json!({
                        "temperature": period.temperature,
                        "unit": period.temperature_unit,
                        "forecast": period.short_forecast,
                        "location": self.location,
                        "office": self.office,
                    })),
                )
            })
            .collect();

        Ok(FetchOutput {
            entries,
            fetch_errors: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_central_park_defaults() {
        let feed = NwsFeed::central_park();
        assert_eq!(feed.name(), "nws_weather");
        assert_eq!(feed.office, "OKX");
        assert_eq!(feed.grid_x, 33);
        assert_eq!(feed.grid_y, 37);
        assert_eq!(feed.location, "NYC");
    }

    #[test]
    fn test_custom_location() {
        let feed = NwsFeed::new("LOT".into(), 75, 73, "Chicago".into());
        assert_eq!(feed.location, "Chicago");
        assert_eq!(feed.office, "LOT");
    }
}
