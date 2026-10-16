/* settings.rs
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

//! Typed access to the application's GSettings.
//!
//! Replaces the hand-rolled `~/.local/share/loci/map_profile` file and adds
//! the keys needed for the map position, units and (later) search history.

use gtk::gio;
use gtk::prelude::*;

use crate::util::Units;

const APP_ID: &str = "io.github.nico359.loci";

fn settings() -> gio::Settings {
    gio::Settings::new(APP_ID)
}

/// The persisted map profile, either `"online"` or `"offline"`.
pub(crate) fn map_profile() -> String {
    if settings().string("map-profile").as_str() == "offline" {
        "offline".to_string()
    } else {
        "online".to_string()
    }
}

pub(crate) fn set_map_profile(profile: &str) {
    let _ = settings().set_string("map-profile", profile);
}

/// The last viewed map position as `(lat, lon)`, if one was ever stored.
pub(crate) fn last_position() -> Option<(f64, f64)> {
    let raw = settings().string("last-location");
    let (lat, lon) = raw.split_once(',')?;
    Some((lat.trim().parse().ok()?, lon.trim().parse().ok()?))
}

pub(crate) fn set_last_position(lat: f64, lon: f64) {
    let _ = settings().set_string("last-location", &format!("{lat},{lon}"));
}

pub(crate) fn last_zoom() -> f64 {
    let zoom = settings().double("last-zoom");
    if (1.0..=20.0).contains(&zoom) {
        zoom
    } else {
        12.0
    }
}

pub(crate) fn set_last_zoom(zoom: f64) {
    let _ = settings().set_double("last-zoom", zoom);
}

/// The units preference as stored: `"auto"`, `"metric"` or `"imperial"`.
pub(crate) fn units_preference() -> String {
    settings().string("units").to_string()
}

pub(crate) fn set_units_preference(preference: &str) {
    let _ = settings().set_string("units", preference);
}

/// The resolved distance units, applying the locale when the preference is
/// `"auto"`.
pub(crate) fn units() -> Units {
    match units_preference().as_str() {
        "metric" => Units::Metric,
        "imperial" => Units::Imperial,
        _ => locale_units(),
    }
}

/// Units implied by the locale. Only a few countries still use imperial /
/// US customary units for road distances.
fn locale_units() -> Units {
    const IMPERIAL_COUNTRIES: [&str; 3] = ["US", "LR", "MM"];
    for name in gtk::glib::language_names() {
        if let Some(country) = name.split('_').nth(1) {
            if IMPERIAL_COUNTRIES.contains(&country.to_ascii_uppercase().as_str()) {
                return Units::Imperial;
            }
        }
    }
    Units::Metric
}
