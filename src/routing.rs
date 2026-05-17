/* routing.rs
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

use ferrostar::routing_adapters::osrm::OsrmResponseParser;
use ferrostar::routing_adapters::RouteResponseParser;
use ferrostar::models::Route;

/// Base URL for the OSRM routing service.
pub const DEFAULT_OSRM_BASE_URL: &str = "https://routing.openstreetmap.de";

/// Request a route from `origin` to `destination` using OSRM.
///
/// `origin` and `destination` are `(lat, lon)` pairs.
/// The `profile` is the OSRM vehicle profile: `"car"`, `"bike"`, or `"foot"`.
/// Returns the first Route on success.
pub fn get_route(
    origin: (f64, f64),
    destination: (f64, f64),
    osrm_base_url: &str,
    profile: &str,
) -> Option<Route> {
    // OSRM expects coordinates in lon,lat order.
    let url = format!(
        "{}/routed-{}/route/v1/driving/{},{};{},{}?overview=full&steps=true&annotations=duration,distance",
        osrm_base_url.trim_end_matches('/'),
        profile,
        origin.1, origin.0,
        destination.1, destination.0,
    );

    eprintln!("[routing] GET {url}");

    let client = reqwest::blocking::Client::new();
    let bytes = match client.get(&url).send() {
        Ok(resp) => {
            let status = resp.status();
            let bytes = resp.bytes().ok()?;
            if !status.is_success() {
                eprintln!("[routing] OSRM error {status}: {}", String::from_utf8_lossy(&bytes));
                return None;
            }
            bytes
        }
        Err(e) => {
            eprintln!("[routing] HTTP error: {e}");
            return None;
        }
    };

    eprintln!("[routing] response {} bytes", bytes.len());

    // Standard OSRM uses polyline precision 5.
    match OsrmResponseParser::new(5).parse_response(bytes.to_vec()) {
        Ok(routes) => {
            eprintln!("[routing] parsed {} route(s)", routes.len());
            routes.into_iter().next()
        }
        Err(e) => {
            eprintln!("[routing] parse error: {e:?}");
            None
        }
    }
}
