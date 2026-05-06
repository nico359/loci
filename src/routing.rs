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

use ferrostar::routing_adapters::{
    RouteAdapter, RouteRequest, WellKnownRouteProvider,
};
use ferrostar::models::{GeographicCoordinate, Route, UserLocation, Waypoint, WaypointKind};
use std::time::SystemTime;

pub const DEFAULT_VALHALLA_URL: &str = "https://valhalla1.openstreetmap.de/route";

/// Request a route from `origin` to `destination` using Valhalla.
/// Returns the first Route on success.
pub fn get_route(
    origin: (f64, f64),
    destination: (f64, f64),
    valhalla_url: &str,
    profile: &str,
) -> Option<Route> {
    let adapter = RouteAdapter::from_well_known_route_provider(
        WellKnownRouteProvider::Valhalla {
            endpoint_url: valhalla_url.to_string(),
            profile: profile.to_string(),
            options_json: None,
        },
    )
    .ok()?;

    let user_location = UserLocation {
        coordinates: GeographicCoordinate {
            lat: origin.0,
            lng: origin.1,
        },
        horizontal_accuracy: 0.0,
        course_over_ground: None,
        timestamp: SystemTime::now(),
        speed: None,
    };

    let waypoints = vec![Waypoint {
        coordinate: GeographicCoordinate {
            lat: destination.0,
            lng: destination.1,
        },
        kind: WaypointKind::Break,
        properties: None,
    }];

    let route_request = adapter.generate_request(user_location, waypoints).ok()?;

    let response_bytes = match route_request {
        RouteRequest::HttpPost { url, headers, body } => {
            let client = reqwest::blocking::Client::new();
            let mut req = client.post(&url).body(body);
            for (k, v) in &headers {
                req = req.header(k, v);
            }
            req.send().ok()?.bytes().ok()?
        }
        RouteRequest::HttpGet { url, headers } => {
            let client = reqwest::blocking::Client::new();
            let mut req = client.get(&url);
            for (k, v) in &headers {
                req = req.header(k, v);
            }
            req.send().ok()?.bytes().ok()?
        }
    };

    adapter.parse_response(response_bytes.to_vec()).ok()?.into_iter().next()
}
