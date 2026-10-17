/* style.rs
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

//! The vector map style handed to libshumate.
//!
//! Built as a JSON string from the tile and glyph URLs of the active map
//! source, so the same style serves both the online (OpenFreeMap) and offline
//! (OSM Scout Server) profiles.

/// Builds the complete Mapbox GL style JSON using the given tile and glyph URLs.
pub(crate) fn build_style_json(tile_url: &str, glyph_url: &str) -> String {
    serde_json::json!({
        "version": 8,
        "name": "Loci",
        "sources": {
            "openmaptiles": {
                "type": "vector",
                "tiles": [tile_url],
                "minzoom": 0,
                "maxzoom": 14
            }
        },
        "glyphs": glyph_url,
        "layers": [
            // --- Background & land ---
            {"id": "background", "type": "background",
             "paint": {"background-color": "#f0ebe3"}},
            {"id": "landcover-grass", "type": "fill",
             "source": "openmaptiles", "source-layer": "landcover",
             "filter": ["in", "class", "grass", "meadow", "park"],
             "paint": {"fill-color": "#d8eeca"}},
            {"id": "landcover-forest", "type": "fill",
             "source": "openmaptiles", "source-layer": "landcover",
             "filter": ["==", "class", "forest"],
             "paint": {"fill-color": "#c0ddb0"}},
            // --- Landuse ---
            {"id": "landuse-residential", "type": "fill",
             "source": "openmaptiles", "source-layer": "landuse",
             "filter": ["==", "class", "residential"],
             "paint": {"fill-color": "#e8e0d8"}},
            {"id": "landuse-commercial", "type": "fill",
             "source": "openmaptiles", "source-layer": "landuse",
             "filter": ["==", "class", "commercial"],
             "paint": {"fill-color": "#f0e8d8"}},
            {"id": "landuse-industrial", "type": "fill",
             "source": "openmaptiles", "source-layer": "landuse",
             "filter": ["==", "class", "industrial"],
             "paint": {"fill-color": "#ded8cc"}},
            {"id": "landuse-park", "type": "fill",
             "source": "openmaptiles", "source-layer": "landuse",
             "filter": ["in", "class", "park", "pitch"],
             "paint": {"fill-color": "#d0e8c0"}},
            // --- Water ---
            {"id": "water-fill", "type": "fill",
             "source": "openmaptiles", "source-layer": "water",
             "paint": {"fill-color": "#a8d4f0"}},
            {"id": "waterway", "type": "line",
             "source": "openmaptiles", "source-layer": "waterway",
             "paint": {"line-color": "#a8d4f0",
                       "line-width": ["interpolate", ["linear"], ["zoom"],
                                      10, 1, 14, 3]}},
            // --- Buildings ---
            {"id": "building", "type": "fill",
             "source": "openmaptiles", "source-layer": "building",
             "minzoom": 16,
             "paint": {"fill-color": "#dbd5cc"}},
            // --- Roads ---
            {"id": "road-path", "type": "line",
             "source": "openmaptiles", "source-layer": "transportation",
             "filter": ["in", "class", "path", "track"],
             "layout": {"line-cap": "round", "line-join": "round"},
             "paint": {"line-color": "#d0c8c0", "line-width": 1,
                       "line-dasharray": [2, 2]}},
            {"id": "road-minor-casing", "type": "line",
             "source": "openmaptiles", "source-layer": "transportation",
             "filter": ["in", "class", "minor", "service"],
             "layout": {"line-cap": "round", "line-join": "round", "line-sort-key": -1},
             "paint": {"line-color": "#d8d0c8",
                       "line-width": ["interpolate", ["linear"], ["zoom"],
                                      13, 1.5, 16, 5]}},
            {"id": "road-minor", "type": "line",
             "source": "openmaptiles", "source-layer": "transportation",
             "filter": ["in", "class", "minor", "service"],
             "layout": {"line-cap": "round", "line-join": "round"},
             "paint": {"line-color": "#f0ece4",
                       "line-width": ["interpolate", ["linear"], ["zoom"],
                                      13, 0.5, 16, 3]}},
            {"id": "road-secondary-casing", "type": "line",
             "source": "openmaptiles", "source-layer": "transportation",
             "filter": ["in", "class", "secondary", "tertiary"],
             "layout": {"line-cap": "round", "line-join": "round", "line-sort-key": -2},
             "paint": {"line-color": "#d8d0b0",
                       "line-width": ["interpolate", ["linear"], ["zoom"],
                                      10, 1.5, 14, 4, 16, 8]}},
            {"id": "road-secondary", "type": "line",
             "source": "openmaptiles", "source-layer": "transportation",
             "filter": ["in", "class", "secondary", "tertiary"],
             "layout": {"line-cap": "round", "line-join": "round"},
             "paint": {"line-color": "#f8f4e8",
                       "line-width": ["interpolate", ["linear"], ["zoom"],
                                      10, 0.5, 14, 2.5, 16, 6]}},
            {"id": "road-primary-casing", "type": "line",
             "source": "openmaptiles", "source-layer": "transportation",
             "filter": ["in", "class", "primary", "trunk"],
             "layout": {"line-cap": "round", "line-join": "round", "line-sort-key": -3},
             "paint": {"line-color": "#d4b86a",
                       "line-width": ["interpolate", ["linear"], ["zoom"],
                                      8, 1.5, 12, 3, 15, 5.5, 17, 10]}},
            {"id": "road-primary", "type": "line",
             "source": "openmaptiles", "source-layer": "transportation",
             "filter": ["in", "class", "primary", "trunk"],
             "layout": {"line-cap": "round", "line-join": "round"},
             "paint": {"line-color": "#fce8a0",
                       "line-width": ["interpolate", ["linear"], ["zoom"],
                                      8, 0.5, 12, 1.5, 15, 3.5, 17, 8]}},
            {"id": "road-motorway-casing", "type": "line",
             "source": "openmaptiles", "source-layer": "transportation",
             "filter": ["==", "class", "motorway"],
             "layout": {"line-cap": "round", "line-join": "round", "line-sort-key": -4},
             "paint": {"line-color": "#c87d30",
                       "line-width": ["interpolate", ["linear"], ["zoom"],
                                      6, 1.5, 10, 3, 15, 6, 17, 11]}},
            {"id": "road-motorway", "type": "line",
             "source": "openmaptiles", "source-layer": "transportation",
             "filter": ["==", "class", "motorway"],
             "layout": {"line-cap": "round", "line-join": "round"},
             "paint": {"line-color": "#e8943a",
                       "line-width": ["interpolate", ["linear"], ["zoom"],
                                      6, 0.5, 10, 1.5, 15, 4, 17, 9]}},
            // --- Labels ---
            {"id": "road-name", "type": "symbol",
             "source": "openmaptiles", "source-layer": "transportation_name",
             "minzoom": 14,
             "layout": {
                 "text-field": ["get", "name"],
                 "text-size": 11,
                 "text-font": ["Noto Sans Regular"],
                 "symbol-placement": "line",
                 "text-max-angle": 30
             },
             "paint": {"text-color": "#555",
                       "text-halo-color": "#fff",
                       "text-halo-width": 1}},
            // Important POIs (rank 1-2: stations, hospitals, airports) from zoom 13
            {"id": "poi-important", "type": "symbol",
             "source": "openmaptiles", "source-layer": "poi",
             "minzoom": 13,
             "filter": ["<=", "rank", 2],
             "layout": {
                 "text-field": ["get", "name"],
                 "text-size": 11,
                 "text-font": ["Noto Sans Bold"],
                 "text-anchor": "top",
                 "text-offset": [0, 0.4],
                 "text-max-width": 8
             },
             "paint": {"text-color": "#333",
                       "text-halo-color": "#fff",
                       "text-halo-width": 1.5}},
            // General POIs (supermarkets, restaurants, shops, etc.) from zoom 15
            {"id": "poi-label", "type": "symbol",
             "source": "openmaptiles", "source-layer": "poi",
             "minzoom": 15,
             "filter": ["<=", "rank", 6],
             "layout": {
                 "text-field": ["get", "name"],
                 "text-size": 11,
                 "text-font": ["Noto Sans Regular"],
                 "text-anchor": "top",
                 "text-offset": [0, 0.4],
                 "text-max-width": 7
             },
             "paint": {"text-color": "#555",
                       "text-halo-color": "#fff",
                       "text-halo-width": 1}},

            {"id": "place-suburb", "type": "symbol",
             "source": "openmaptiles", "source-layer": "place",
             "filter": ["in", "class", "suburb", "quarter", "neighbourhood"],
             "minzoom": 13,
             "layout": {
                 "text-field": ["get", "name"],
                 "text-size": 11,
                 "text-font": ["Noto Sans Italic"],
                 "text-transform": "uppercase"
             },
             "paint": {"text-color": "#888",
                       "text-halo-color": "#f0ebe3",
                       "text-halo-width": 1}},
            {"id": "place-city", "type": "symbol",
             "source": "openmaptiles", "source-layer": "place",
             "filter": ["in", "class", "city", "town", "village"],
             "layout": {
                 "text-field": ["get", "name"],
                 "text-size": ["interpolate", ["linear"], ["zoom"],
                               8, 11, 12, 14],
                 "text-font": ["Noto Sans Bold"]
             },
             "paint": {"text-color": "#333",
                       "text-halo-color": "#fff",
                       "text-halo-width": 2}}
        ]
    }).to_string()
}
