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

#[derive(Clone, Debug, Default)]
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

        let name = props["name"]
            .as_str()
            .or_else(|| props["street"].as_str())
            .unwrap_or("Unknown place")
            .to_owned();

        let subtitle = [
            props["city"].as_str(),
            props["state"].as_str(),
            props["country"].as_str(),
        ]
        .iter()
        .filter_map(|x| *x)
        .collect::<Vec<_>>()
        .join(", ");

        Some(PhotonFeature { name, subtitle, lat, lon })
    }
}

/// Dispatch to the right geocoder depending on the current map profile.
/// Blocking — call from a background thread.
pub fn search_with_profile(query: &str, profile: &str) -> Vec<PhotonFeature> {
    if profile == "offline" {
        search_scout(query)
    } else {
        search(query)
    }
}

/// Search Photon (https://photon.komoot.io) and return up to 5 results.
/// Blocking — call from a background thread.
pub fn search(query: &str) -> Vec<PhotonFeature> {
    let client = reqwest::blocking::Client::new();
    let response = client
        .get("https://photon.komoot.io/api/")
        .query(&[("q", query), ("limit", "5")])
        .send();

    response
        .and_then(|r| r.text())
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v["features"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(PhotonFeature::from_value)
        .collect()
}

/// Search OSM Scout Server's local geocoder and return up to 5 results.
/// Blocking — call from a background thread.
fn search_scout(query: &str) -> Vec<PhotonFeature> {
    let client = reqwest::blocking::Client::new();
    let response = client
        .get("http://localhost:8553/v1/search")
        .query(&[("search", query), ("limit", "5")])
        .send();

    let items: Vec<serde_json::Value> = response
        .and_then(|r| r.json::<serde_json::Value>())
        .ok()
        .and_then(|v| {
            // Scout Server wraps results under "results" key in some versions,
            // or returns a bare array. Handle both.
            if v.is_array() {
                v.as_array().cloned()
            } else {
                v["results"].as_array().cloned()
            }
        })
        .unwrap_or_default();

    items
        .iter()
        .filter_map(|item| {
            let lat = item["lat"].as_f64()?;
            let lon = item["lng"].as_f64()?;
            let name = item["title"].as_str().unwrap_or("").to_owned();
            if name.is_empty() { return None; }
            let subtitle = item["admin_region"].as_str().unwrap_or("").to_owned();
            Some(PhotonFeature { name, subtitle, lat, lon })
        })
        .collect()
}
