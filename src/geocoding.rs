/* geocoding.rs
 *
 * Copyright 2026 furios
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use crate::util::haversine;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PhotonFeature {
    pub name: String,
    pub subtitle: String,
    pub lat: f64,
    pub lon: f64,
}

impl PhotonFeature {
    fn from_value(feature: &serde_json::Value) -> Option<Self> {
        let coords = feature["geometry"]["coordinates"].as_array()?;
        let lon = coords.first()?.as_f64()?;
        let lat = coords.get(1)?.as_f64()?;
        let props = &feature["properties"];

        let street_line = match (props["street"].as_str(), props["housenumber"].as_str()) {
            (Some(street), Some(number)) => Some(format!("{street} {number}")),
            (Some(street), None) => Some(street.to_owned()),
            _ => None,
        };

        let name = props["name"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| street_line.clone())
            .unwrap_or_else(|| "Unknown place".to_owned());

        // Lead with the address (unless it is already the name), then the
        // administrative context, so addresses are distinguishable from POIs.
        let mut parts: Vec<&str> = Vec::new();
        if let Some(line) = street_line.as_deref() {
            if line != name {
                parts.push(line);
            }
        }
        for key in ["city", "state", "country"] {
            if let Some(value) = props[key].as_str() {
                parts.push(value);
            }
        }

        Some(PhotonFeature {
            name,
            subtitle: parts.join(", "),
            lat,
            lon,
        })
    }
}

/// Search Photon (https://photon.komoot.io) and return up to 5 results.
/// Blocking — call from a background thread.
pub fn search(query: &str, near: Option<(f64, f64)>) -> Vec<PhotonFeature> {
    let client = crate::routing::http_client();

    let mut params: Vec<(&str, String)> =
        vec![("q", query.to_owned()), ("limit", "6".to_owned())];
    if let Some((lat, lon)) = near {
        params.push(("lat", lat.to_string()));
        params.push(("lon", lon.to_string()));
    }

    let results = client
        .get("https://photon.komoot.io/api/")
        .query(&params)
        .send()
        .and_then(|r| r.json::<serde_json::Value>())
        .ok()
        .and_then(|v| v["features"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(PhotonFeature::from_value)
        .collect();

    dedup(results).into_iter().take(5).collect()
}

/// Search OSM Scout Server's local geocoder and return up to 5 results.
/// Blocking — call from a background thread.
///
/// libosmscout's location search ignores the location-bias parameters (only
/// Scout's geocoder-nlp backend uses them) and ranks globally, so we fetch a
/// large candidate set and rank by distance ourselves. Its parser also does
/// not fold the German sharp s, so `ß` is normalized to `ss`.
pub(crate) fn search_scout(query: &str, near: Option<(f64, f64)>) -> Vec<PhotonFeature> {
    let client = crate::routing::http_client();
    let normalized = normalize_query(query);

    // Scout ranks globally and truncates to `limit`, so the nearest match can
    // sit beyond the first page: "Burger King" found its nearest at 39 km with
    // 300 results, but at 15 km once the full match set was requested. Start
    // with a moderate page and widen only when the page was truncated and
    // nothing is close by, so common local queries stay fast.
    const PAGE: usize = 600;
    const WIDE: usize = 2000;
    const CLOSE_M: f64 = 5_000.0;

    let mut items = scout_fetch(client, &normalized, near, PAGE);
    if let Some((lat, lon)) = near {
        if items.len() >= PAGE {
            let nearest = items
                .iter()
                .filter_map(|item| {
                    let ilat = item["lat"].as_f64()?;
                    let ilon = item["lng"].as_f64()?;
                    Some(haversine(lat, lon, ilat, ilon))
                })
                .fold(f64::INFINITY, f64::min);
            if nearest > CLOSE_M {
                let wide = scout_fetch(client, &normalized, near, WIDE);
                if wide.len() > items.len() {
                    items = wide;
                }
            }
        }
    }

    let mut results: Vec<PhotonFeature> = items
        .iter()
        .filter_map(|item| {
            let lat = item["lat"].as_f64()?;
            let lon = item["lng"].as_f64()?;
            let name = item["title"].as_str().unwrap_or("").to_owned();
            if name.is_empty() { return None; }
            let subtitle = item["admin_region"].as_str().unwrap_or("").to_owned();
            Some(PhotonFeature { name, subtitle, lat, lon })
        })
        .collect();

    // Client-side distance ranking is the only bias available for libosmscout.
    if let Some((lat, lon)) = near {
        results.sort_by(|a, b| {
            haversine(lat, lon, a.lat, a.lon)
                .partial_cmp(&haversine(lat, lon, b.lat, b.lon))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    dedup(results).into_iter().take(5).collect()
}

/// Fetch a Scout search page, preferring v2 and falling back to v1.
fn scout_fetch(
    client: &reqwest::blocking::Client,
    query: &str,
    near: Option<(f64, f64)>,
    limit: usize,
) -> Vec<serde_json::Value> {
    let mut params: Vec<(&str, String)> = vec![
        ("search", query.to_owned()),
        ("limit", limit.to_string()),
    ];
    if let Some((lat, lon)) = near {
        params.push(("lat", lat.to_string()));
        params.push(("lng", lon.to_string()));
        params.push(("zoom", "16".to_owned()));
        params.push(("importance", "0.75".to_owned()));
    }

    for endpoint in [
        "http://localhost:8553/v2/search",
        "http://localhost:8553/v1/search",
    ] {
        if let Ok(response) = client.get(endpoint).query(&params).send() {
            if let Ok(value) = response.json::<serde_json::Value>() {
                let items = scout_items(&value);
                if !items.is_empty() {
                    return items;
                }
            }
        }
    }
    Vec::new()
}

/// Extract the result list from either Scout response shape: a bare array,
/// v2's `result`, or the older `results` key.
fn scout_items(value: &serde_json::Value) -> Vec<serde_json::Value> {
    if let Some(items) = value.as_array() {
        items.clone()
    } else if let Some(items) = value["result"].as_array() {
        items.clone()
    } else {
        value["results"].as_array().cloned().unwrap_or_default()
    }
}

/// libosmscout's search parser does not fold the German sharp s.
fn normalize_query(query: &str) -> String {
    query.replace('ß', "ss").replace('ẞ', "SS")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_folds_sharp_s() {
        assert_eq!(normalize_query("Hauptstraße"), "Hauptstrasse");
        assert_eq!(normalize_query("HAUPTSTRASSE"), "HAUPTSTRASSE");
        assert_eq!(normalize_query("Straße 5"), "Strasse 5");
    }
}

/// Drop near-duplicate results that describe the same place. Photon frequently
/// returns the same street a few tens of metres apart, and a POI sitting on it.
fn dedup(list: Vec<PhotonFeature>) -> Vec<PhotonFeature> {
    let mut out: Vec<PhotonFeature> = Vec::with_capacity(list.len());
    for feature in list {
        let duplicate = out.iter().any(|kept| {
            kept.name == feature.name
                && haversine(kept.lat, kept.lon, feature.lat, feature.lon) < 100.0
        });
        if !duplicate {
            out.push(feature);
        }
    }
    out
}
