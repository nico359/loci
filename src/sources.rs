/* sources.rs
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

//! Backend selection for routing and geocoding.
//!
//! The rest of the app asks for a capability without caring whether it comes
//! from an online service or the offline OSM Scout Server. Keeping that choice
//! in one place is what makes it possible to add an offline stack, or mix
//! per-capability fallbacks later, without touching the UI.

use ferrostar::models::Route;

use crate::geocoding::{self, PhotonFeature};
use crate::routing;

/// Which set of backends is in use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapProfile {
    #[default]
    Online,
    Offline,
}

impl MapProfile {
    pub fn from_setting(value: &str) -> Self {
        if value == "offline" {
            Self::Offline
        } else {
            Self::Online
        }
    }

    pub fn setting_value(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
        }
    }

    pub fn is_offline(self) -> bool {
        matches!(self, Self::Offline)
    }
}

// Service endpoints per backend. Grouped here so the UI does not need to know
// which provider a profile talks to.
pub const OFFLINE_TILE_URL: &str =
    "http://localhost:8553/v1/mbgl/tile?z={z}&x={x}&y={y}";
pub const OFFLINE_GLYPH_URL: &str =
    "http://localhost:8553/v1/mbgl/glyphs?stack={fontstack}&range={range}";
pub const ONLINE_TILEJSON_URL: &str = "https://tiles.openfreemap.org/planet";
pub const ONLINE_GLYPH_URL: &str =
    "https://tiles.openfreemap.org/fonts/{fontstack}/{range}.pbf";
pub const ONLINE_TILE_FALLBACK: &str =
    "https://tileserver.gnome.org/data/v3/{z}/{x}/{y}.pbf";

/// Compute a route with the selected backend.
/// Blocking — call from a background thread.
pub fn route(
    profile: MapProfile,
    origin: (f64, f64),
    destination: (f64, f64),
    travel_mode: &str,
) -> Option<Route> {
    match profile {
        MapProfile::Online => {
            routing::get_route(origin, destination, routing::DEFAULT_OSRM_BASE_URL, travel_mode)
        }
        MapProfile::Offline => routing::get_route_valhalla(origin, destination, travel_mode),
    }
}

/// Geocode a query with the selected backend.
/// Blocking — call from a background thread.
pub fn search(
    profile: MapProfile,
    query: &str,
    near: Option<(f64, f64)>,
) -> Vec<PhotonFeature> {
    match profile {
        MapProfile::Online => geocoding::search(query, near),
        MapProfile::Offline => geocoding::search_scout(query),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_setting_round_trip() {
        for profile in [MapProfile::Online, MapProfile::Offline] {
            assert_eq!(MapProfile::from_setting(profile.setting_value()), profile);
        }
        // Unknown persisted values fall back to online.
        assert_eq!(MapProfile::from_setting("nonsense"), MapProfile::Online);
    }
}
