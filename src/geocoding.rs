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
/// Uses the v2 endpoint (geocoder-nlp), which parses free-form multi-word
/// addresses better than v1, and biases results towards `near`.
pub(crate) fn search_scout(query: &str, near: Option<(f64, f64)>) -> Vec<PhotonFeature> {
    let client = crate::routing::http_client();

    let mut params: Vec<(&str, String)> = vec![
        ("search", query.to_owned()),
        ("limit", "6".to_owned()),
    ];
    if let Some((lat, lon)) = near {
        params.push(("lat", lat.to_string()));
        params.push(("lng", lon.to_string()));
        params.push(("zoom", "16".to_owned()));
        params.push(("importance", "0.75".to_owned()));
    }

    // Prefer v2; fall back to v1 on older servers or when v2 yields nothing.
    let mut items = Vec::new();
    for endpoint in [
        "http://localhost:8553/v2/search",
        "http://localhost:8553/v1/search",
    ] {
        if let Ok(response) = client.get(endpoint).query(&params).send() {
            if let Ok(value) = response.json::<serde_json::Value>() {
                items = scout_items(&value);
                if !items.is_empty() {
                    break;
                }
            }
        }
    }

    let results = items
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

    dedup(results).into_iter().take(5).collect()
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
