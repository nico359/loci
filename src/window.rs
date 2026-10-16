/* window.rs
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

use gtk::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use shumate::prelude::*;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use crate::geocoding::PhotonFeature;
use crate::settings;
use crate::style::build_style_json;
use crate::util::{capitalize, compute_bearing, format_distance, format_duration, haversine, Units};

/// A deviation detector that only checks the *current* route step (index 0 of remaining_steps).
///
/// Ferrostar's built-in `StaticThreshold` checks ALL remaining steps, so if you're within
/// `max_acceptable_deviation` metres of ANY future step (common on urban grids with parallel
/// streets), it reports `NoDeviation` even when you've clearly departed from the route.
/// By checking only the current step we avoid these false negatives.
struct CurrentStepDeviationDetector {
    max_acceptable_deviation: f64,
}

impl ferrostar::deviation_detection::RouteDeviationDetector for CurrentStepDeviationDetector {
    fn check_route_deviation(
        &self,
        _route: ferrostar::models::Route,
        trip_state: ferrostar::navigation_controller::models::TripState,
    ) -> ferrostar::deviation_detection::RouteDeviation {
        use ferrostar::deviation_detection::{DeviationKind, RouteDeviation};
        use ferrostar::navigation_controller::models::TripState;
        use geo::Point;

        if let TripState::Navigating { user_location, remaining_steps, .. } = trip_state {
            if let Some(step) = remaining_steps.first() {
                let point = Point::from(user_location);
                let line: geo::LineString = step.geometry.iter()
                    .map(|c| geo::coord! { x: c.lng, y: c.lat })
                    .collect();
                if let Some(dist) = ferrostar::algorithms::deviation_from_line(&point, &line) {
                    if dist > self.max_acceptable_deviation {
                        return RouteDeviation::Deviation {
                            kind: DeviationKind::CompletelyOffRoute { deviation_from_route_line: dist },
                        };
                    }
                }
            }
        }
        RouteDeviation::NoDeviation
    }
}

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/nico359/loci/window.ui")]
    pub struct LociWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub map: TemplateChild<shumate::SimpleMap>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub location_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub nav_banner_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub nav_instruction_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub nav_distance_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub nav_maneuver_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub nav_stop_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub nav_remaining_distance_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub nav_remaining_time_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub zoom_in_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub zoom_out_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub route_preview_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub route_dest_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub route_distance_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub route_time_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub route_start_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub route_cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub route_mode_car_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub route_mode_bike_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub route_mode_foot_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub directions_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub planner_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub planner_close_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub planner_from_entry: TemplateChild<gtk::Entry>,
        #[template_child]
        pub planner_to_entry: TemplateChild<gtk::Entry>,
        #[template_child]
        pub planner_go_button: TemplateChild<gtk::Button>,

        // Place info panel (shown when a map symbol is tapped)
        #[template_child]
        pub place_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub place_name_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub place_subtitle_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub place_navigate_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub place_close_button: TemplateChild<gtk::Button>,

        pub marker_layer: RefCell<Option<shumate::MarkerLayer>>,
        pub location_layer: RefCell<Option<shumate::MarkerLayer>>,
        pub route_layer: RefCell<Option<shumate::PathLayer>>,
        pub current_location: RefCell<Option<(f64, f64)>>,
        pub current_results: RefCell<Vec<PhotonFeature>>,

        // Planner state: geocoded coords for from/to entries
        pub planner_from_coords: RefCell<Option<(f64, f64)>>,
        pub planner_to_coords: RefCell<Option<(f64, f64)>>,
        pub planner_to_name: RefCell<String>,

        // Pending route: stored after fetch, before user taps "Start"
        pub pending_route: RefCell<Option<ferrostar::models::Route>>,
        pub pending_origin: RefCell<Option<(f64, f64)>>,

        // Currently selected routing profile ("car", "bike", "foot")
        pub current_profile: RefCell<String>,
        // Cached preview destination for re-fetching when mode changes
        pub preview_destination: RefCell<Option<(f64, f64)>>,
        pub preview_dest_name: RefCell<String>,

        // Navigation state shared between the GPS thread and the main loop timer
        pub nav_controller: RefCell<Option<std::sync::Arc<ferrostar::navigation_controller::NavigationController>>>,
        pub nav_state: std::sync::Arc<std::sync::Mutex<Option<ferrostar::navigation_controller::models::NavState>>>,
        // Last known position used to compute bearing when GPS heading is unavailable
        pub last_nav_pos: RefCell<Option<(f64, f64)>>,
        // Navigation camera state: whether the map follows the user, plus the
        // last viewport values we set (used to detect a manual pan or zoom).
        pub following: std::cell::Cell<bool>,
        pub last_set_location: std::cell::Cell<Option<(f64, f64)>>,
        pub last_set_zoom: std::cell::Cell<Option<f64>>,
        // Zoom derived from the current speed, eased towards while following.
        pub nav_zoom_target: std::cell::Cell<Option<f64>>,
        // Timestamp of the previous navigation fix, for a speed fallback.
        pub last_nav_fix_time: std::cell::Cell<Option<std::time::Instant>>,
        // The 60 fps navigation render timer, installed only while navigating.
        pub render_timer: RefCell<Option<glib::SourceId>>,
        // Ring buffer of recent positions (up to 4) for vector-averaged heading
        pub recent_positions: RefCell<Vec<(f64, f64)>>,
        // Screen idle inhibit — held during navigation, dropped to release
        pub idle_inhibit: RefCell<Option<u32>>,

        // Persistent location dot and animation state for smooth inter-fix movement
        pub location_marker: RefCell<Option<shumate::Marker>>,
        pub anim_from: RefCell<Option<(f64, f64)>>,
        pub anim_to: RefCell<Option<(f64, f64)>>,
        pub anim_start: RefCell<Option<std::time::Instant>>,
        // Linear extrapolation buffer: two most-recent GPS fixes with receipt timestamps.
        // The render timer projects the dot forward beyond the latest fix using the velocity
        // vector between these two samples (CoMaps-style), eliminating the ~1 s display lag.
        pub extrap_fix0: RefCell<Option<(f64, f64, std::time::Instant)>>,
        pub extrap_fix1: RefCell<Option<(f64, f64, std::time::Instant)>>,
        // Heading animation (degrees) for smooth map rotation during navigation
        pub heading_from: RefCell<Option<f64>>,
        pub heading_to: RefCell<Option<f64>>,
        pub heading_anim_start: RefCell<Option<std::time::Instant>>,
        // Duration (seconds) for the current heading animation, computed proportionally
        // to the angular delta so small corrections finish quickly (CoMaps-style).
        pub heading_anim_dur: std::cell::Cell<f64>,

        // Rerouting — destination stored so we can re-request a route when off-route
        pub nav_destination: RefCell<Option<(f64, f64)>>,
        pub last_reroute_at: RefCell<Option<std::time::Instant>>,
        pub is_rerouting: std::cell::Cell<bool>,
        pub reroute_result: std::sync::Arc<std::sync::Mutex<Option<ferrostar::models::Route>>>,

        // Last tapped place from a symbol-clicked event
        pub tapped_place_coords: RefCell<Option<(f64, f64)>>,
        pub tapped_place_name: RefCell<String>,

        // Map source profile: "online" (OpenFreeMap) or "offline" (OSM Scout Server)
        pub map_profile: RefCell<String>,
        // Distance units, refreshed from settings at startup and on change
        pub units: std::cell::Cell<Units>,
        // Totals of the route currently shown in the preview panel, kept so the
        // labels can be reformatted when the units change.
        pub preview_total_distance: std::cell::Cell<f64>,
        pub preview_total_duration: std::cell::Cell<f64>,
        // Whether overlay layers have been added to the map (done once after first renderer)
        pub map_initialized: std::cell::Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LociWindow {
        const NAME: &'static str = "LociWindow";
        type Type = super::LociWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            shumate::SimpleMap::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for LociWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.units.set(settings::units());
            self.obj().setup_actions();
            self.obj().setup_map();
            self.obj().setup_geocoding();
            self.obj().setup_planner();
            self.obj().setup_place_panel();
            self.obj().setup_location();
            self.obj().setup_navigation();
        }
    }

    impl WidgetImpl for LociWindow {}
    impl WindowImpl for LociWindow {}
    impl ApplicationWindowImpl for LociWindow {}
    impl AdwApplicationWindowImpl for LociWindow {}
}

glib::wrapper! {
    pub struct LociWindow(ObjectSubclass<imp::LociWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl LociWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }

    fn setup_map(&self) {
        let imp = self.imp();

        if !shumate::VectorRenderer::is_supported() {
            glib::g_warning!("loci", "ERROR: libshumate was compiled without vector tile support");
            return;
        }

        imp.zoom_in_button.connect_clicked({
            let map = imp.map.clone();
            move |_| {
                if let Some(viewport) = map.viewport() {
                    viewport.set_zoom_level((viewport.zoom_level() + 1.0).min(20.0));
                }
            }
        });
        imp.zoom_out_button.connect_clicked({
            let map = imp.map.clone();
            move |_| {
                if let Some(viewport) = map.viewport() {
                    viewport.set_zoom_level((viewport.zoom_level() - 1.0).max(0.0));
                }
            }
        });

        // Remember the map position so the next launch opens where we left off.
        let window_weak = self.downgrade();
        self.connect_close_request(move |_| {
            if let Some(window) = window_weak.upgrade() {
                if let Some(viewport) = window.imp().map.viewport() {
                    settings::set_last_position(viewport.latitude(), viewport.longitude());
                    settings::set_last_zoom(viewport.zoom_level());
                }
            }
            glib::Propagation::Proceed
        });

        self.start_map_source();
    }

    fn setup_actions(&self) {
        let saved_profile = settings::map_profile();
        *self.imp().map_profile.borrow_mut() = saved_profile.clone();

        let map_profile_action = gio::SimpleAction::new_stateful(
            "map-profile",
            Some(glib::VariantTy::STRING),
            &saved_profile.to_variant(),
        );
        map_profile_action.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |action, param| {
                let Some(profile) = param.and_then(|v| v.str()) else { return };
                action.set_state(&profile.to_variant());
                *window.imp().map_profile.borrow_mut() = profile.to_string();
                settings::set_map_profile(profile);
                window.start_map_source();
            }
        ));
        self.add_action(&map_profile_action);

        let units_preference = settings::units_preference();
        let units_action = gio::SimpleAction::new_stateful(
            "units",
            Some(glib::VariantTy::STRING),
            &units_preference.to_variant(),
        );
        units_action.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |action, param| {
                let Some(preference) = param.and_then(|v| v.str()) else { return };
                action.set_state(&preference.to_variant());
                settings::set_units_preference(preference);
                window.apply_units(settings::units());
            }
        ));
        self.add_action(&units_action);
    }

    /// Store the resolved units and refresh the visible route preview labels.
    fn apply_units(&self, units: Units) {
        let imp = self.imp();
        imp.units.set(units);
        if imp.route_preview_revealer.reveals_child() {
            Self::update_route_preview_labels(imp);
        }
    }

    fn update_route_preview_labels(imp: &imp::LociWindow) {
        let units = imp.units.get();
        imp.route_distance_label
            .set_text(&format_distance(imp.preview_total_distance.get(), units));
        imp.route_time_label
            .set_text(&format_duration(imp.preview_total_duration.get()));
    }

    /// Starts loading the map source matching the current profile.
    fn start_map_source(&self) {
        let profile = self.imp().map_profile.borrow().clone();
        if profile == "offline" {
            self.activate_scout_then_apply();
        } else {
            self.start_online_source();
        }
    }

    /// Pings OSM Scout Server via D-Bus (which auto-starts it if not running),
    /// waits for the HTTP endpoint to become ready, then applies the offline renderer.
    fn activate_scout_then_apply(&self) {
        // Trigger D-Bus auto-start and register as a tracked client in one call.
        // Scout's IdleTracker keeps the server running as long as at least one
        // D-Bus client is registered. Calling Connect() registers our unique D-Bus
        // name; Scout then watches NameOwnerChanged and shuts down only after we
        // disconnect — no periodic keep-alive pings needed.
        let conn = gio::bus_get_future(gio::BusType::Session);
        let window = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(bus) = conn.await {
                let _ = bus.call_future(
                    Some("io.github.rinigus.OSMScoutServer"),
                    "/io/github/rinigus/OSMScoutServer",
                    "io.github.rinigus.OSMScoutServer",
                    "Connect",
                    None,
                    None,
                    gio::DBusCallFlags::NONE,
                    10_000,
                ).await;
            }

            // After D-Bus activation, Scout still needs a moment to bind its HTTP port.
            // Poll /v1/activate up to ~5 s (50 × 100 ms).
            let result: Arc<Mutex<Option<bool>>> = Arc::new(Mutex::new(None));
            let result_writer = Arc::clone(&result);
            std::thread::spawn(move || {
                for _ in 0..50 {
                    let ok = reqwest::blocking::get("http://localhost:8553/v1/activate")
                        .map(|r| r.status().is_success())
                        .unwrap_or(false);
                    if ok {
                        *result_writer.lock().unwrap() = Some(true);
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                *result_writer.lock().unwrap() = Some(false);
            });

            glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                if let Some(ok) = result.lock().unwrap().take() {
                    if ok {
                        let json = build_style_json(
                            "http://localhost:8553/v1/mbgl/tile?z={z}&x={x}&y={y}",
                            "http://localhost:8553/v1/mbgl/glyphs?stack={fontstack}&range={range}",
                        );
                        window.apply_renderer(json);
                    } else {
                        window.show_toast("OSM Scout Server could not be started");
                        // Revert menu selection back to online
                        if let Some(action) = window.lookup_action("map-profile") {
                            if let Some(a) = action.downcast_ref::<gio::SimpleAction>() {
                                a.set_state(&"online".to_variant());
                            }
                        }
                        *window.imp().map_profile.borrow_mut() = "online".to_string();
                        settings::set_map_profile("online");
                        window.start_online_source();
                    }
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
        });
    }

    fn show_toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        self.imp().toast_overlay.add_toast(toast);
    }

    /// Fetches the TileJSON from OpenFreeMap on a background thread, then applies the renderer.
    fn start_online_source(&self) {
        let style_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let slot_writer = Arc::clone(&style_slot);
        std::thread::spawn(move || {
            let tile_url = reqwest::blocking::get("https://tiles.openfreemap.org/planet")
                .and_then(|r| r.json::<serde_json::Value>())
                .ok()
                .and_then(|j| j["tiles"][0].as_str().map(|s| s.to_owned()))
                .unwrap_or_else(|| {
                    glib::g_debug!("loci", "[tiles] TileJSON fetch failed, using fallback");
                    "https://tileserver.gnome.org/data/v3/{z}/{x}/{y}.pbf".to_owned()
                });
            let style = build_style_json(
                &tile_url,
                "https://tiles.openfreemap.org/fonts/{fontstack}/{range}.pbf",
            );
            *slot_writer.lock().unwrap() = Some(style);
        });

        let window = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            // Cancel if the user switched to offline before the fetch completed.
            if window.imp().map_profile.borrow().as_str() != "online" {
                return glib::ControlFlow::Break;
            }
            if let Some(json) = style_slot.lock().unwrap().take() {
                window.apply_renderer(json);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    /// Installs a new VectorRenderer and, on the very first call, adds all overlay layers.
    fn apply_renderer(&self, style_json: String) {
        let imp = self.imp();
        match shumate::VectorRenderer::new("loci-tiles", &style_json) {
            Ok(renderer) => {
                imp.map.set_map_source(Some(&renderer));

                if !imp.map_initialized.get() {
                    imp.map_initialized.set(true);

                    let viewport = imp.map.viewport().expect("SimpleMap has no viewport");
                    let (lat, lon) = settings::last_position().unwrap_or((52.5200, 13.4050));
                    viewport.set_zoom_level(settings::last_zoom());
                    viewport.set_location(lat, lon);

                    let marker_layer = shumate::MarkerLayer::new(&viewport);
                    imp.map.add_overlay_layer(&marker_layer);
                    *imp.marker_layer.borrow_mut() = Some(marker_layer);

                    let location_layer = shumate::MarkerLayer::new(&viewport);
                    imp.map.add_overlay_layer(&location_layer);

                    let location_marker = shumate::Marker::new();
                    let dot = gtk::Box::builder().width_request(18).height_request(18).build();
                    dot.add_css_class("location-dot");
                    location_marker.set_child(Some(&dot));
                    location_layer.add_marker(&location_marker);
                    *imp.location_marker.borrow_mut() = Some(location_marker);
                    *imp.location_layer.borrow_mut() = Some(location_layer);

                    let route_layer = shumate::PathLayer::new(&viewport);
                    route_layer.set_stroke_width(5.0);
                    route_layer.set_stroke_color(Some(&gdk::RGBA::new(0.2, 0.5, 1.0, 0.9)));
                    imp.map.add_overlay_layer(&route_layer);
                    *imp.route_layer.borrow_mut() = Some(route_layer);

                    imp.map.connect_symbol_clicked(glib::clone!(
                        #[weak(rename_to = window)]
                        self,
                        move |_, event| {
                            window.on_symbol_tapped(event);
                        }
                    ));
                }
            }
            Err(e) => glib::g_warning!("loci", "VectorRenderer::new error: {e}"),
        }
    }

    fn setup_place_panel(&self) {
        let imp = self.imp();

        imp.place_close_button.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| { window.imp().place_revealer.set_reveal_child(false); }
        ));

        imp.place_navigate_button.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                let imp = window.imp();
                let coords = *imp.tapped_place_coords.borrow();
                let name = imp.tapped_place_name.borrow().clone();
                if let Some((lat, lon)) = coords {
                    imp.place_revealer.set_reveal_child(false);
                    window.request_route_preview((lat, lon), name);
                }
            }
        ));
    }

    fn on_symbol_tapped(&self, event: &shumate::SymbolEvent) {
        use shumate::prelude::LocationExt;

        let source_layer = match event.source_layer() {
            Some(s) => s.to_string(),
            None => return,
        };

        let lat = event.latitude();
        let lon = event.longitude();

        let (name, subtitle) = match source_layer.as_str() {
            "poi" => {
                let name = event.tag("name").map(|s| s.to_string()).unwrap_or_default();
                if name.is_empty() { return; }
                let class = event.tag("class")
                    .or_else(|| event.tag("subclass"))
                    .map(|s| capitalize(s.as_str()))
                    .unwrap_or_default();
                (name, class)
            }
            "place" => {
                let name = event.tag("name").map(|s| s.to_string()).unwrap_or_default();
                if name.is_empty() { return; }
                let class = event.tag("class").map(|s| capitalize(s.as_str())).unwrap_or_default();
                (name, class)
            }
            "transportation_name" => {
                let name = event.tag("name").map(|s| s.to_string()).unwrap_or_default();
                if name.is_empty() { return; }
                let class = event.tag("class").map(|s| capitalize(s.as_str())).unwrap_or_default();
                (name, class)
            }
            "housenumber" => {
                let number = event.tag("housenumber").map(|s| s.to_string()).unwrap_or_default();
                if number.is_empty() { return; }
                (number, "House number".to_string())
            }
            _ => return,
        };

        let imp = self.imp();
        imp.place_name_label.set_text(&name);
        imp.place_subtitle_label.set_text(&subtitle);
        imp.place_subtitle_label.set_visible(!subtitle.is_empty());
        *imp.tapped_place_coords.borrow_mut() = Some((lat, lon));
        *imp.tapped_place_name.borrow_mut() = name.to_owned();

        // Hide other panels to avoid overlap
        imp.route_preview_revealer.set_reveal_child(false);

        imp.place_revealer.set_reveal_child(true);
    }

    fn setup_geocoding(&self) {
        let imp = self.imp();

        // Main search entry: on select, pan map, place marker, fetch route from GPS origin
        let suppress_search = std::rc::Rc::new(std::cell::Cell::new(false));
        attach_geocoding_popover(
            self,
            &imp.search_entry.get(),
            suppress_search, // main search doesn't set_text, flag unused but required
            None, // no next field → dismiss keyboard after selection
            {
                let window_weak = self.downgrade();
                move || {
                    window_weak
                        .upgrade()
                        .map(|w| {
                            let imp = w.imp();
                            (imp.map_profile.borrow().clone(), *imp.current_location.borrow())
                        })
                        .unwrap_or_else(|| ("online".to_string(), None))
                }
            },
            {
                let window_weak = self.downgrade();
                move |feat| {
                    let Some(window) = window_weak.upgrade() else { return };
                    let imp = window.imp();
                    let (lat, lon) = (feat.lat, feat.lon);
                    let dest_name = feat.name.clone();

                    // Pan map and place marker
                    let viewport = imp.map.viewport().expect("no viewport");
                    viewport.set_zoom_level(14.0);
                    viewport.set_location(lat, lon);

                    if let Some(layer) = imp.marker_layer.borrow().as_ref() {
                        layer.remove_all();
                        let marker = shumate::Marker::new();
                        let img = gtk::Image::builder()
                            .icon_name("map-marker-symbolic")
                            .pixel_size(32)
                            .build();
                        img.add_css_class("accent");
                        marker.set_child(Some(&img));
                        marker.set_location(lat, lon);
                        layer.add_marker(&marker);
                    }

                    window.request_route_preview((lat, lon), dest_name);
                }
            },
        );
    }

    fn setup_planner(&self) {
        let imp = self.imp();

        // Close button — untoggle the directions button to collapse the panel
        imp.planner_close_button.connect_clicked({
            let window_weak = self.downgrade();
            move |_| {
                let Some(window) = window_weak.upgrade() else { return };
                let imp = window.imp();
                imp.directions_button.set_active(false);
                imp.planner_revealer.set_reveal_child(false);
            }
        });

        // Toggle planner panel visibility
        imp.directions_button.connect_toggled({
            let window_weak = self.downgrade();
            move |btn| {
                let Some(window) = window_weak.upgrade() else { return };
                let imp = window.imp();
                let active = btn.is_active();
                imp.planner_revealer.set_reveal_child(active);
                // Hide route preview and nav banner when opening planner
                if active {
                    imp.route_preview_revealer.set_reveal_child(false);
                    imp.place_revealer.set_reveal_child(false);
                }
            }
        });

        // From entry: geocode → store planner_from_coords, then move focus to To entry
        let suppress_from = std::rc::Rc::new(std::cell::Cell::new(false));
        attach_geocoding_popover(
            self,
            &imp.planner_from_entry.get(),
            suppress_from.clone(),
            Some(imp.planner_to_entry.get().upcast::<gtk::Widget>()), // focus To next
            {
                let window_weak = self.downgrade();
                move || {
                    window_weak
                        .upgrade()
                        .map(|w| {
                            let imp = w.imp();
                            (imp.map_profile.borrow().clone(), *imp.current_location.borrow())
                        })
                        .unwrap_or_else(|| ("online".to_string(), None))
                }
            },
            {
                let window_weak = self.downgrade();
                let suppress_from = suppress_from.clone();
                move |feat| {
                    let Some(window) = window_weak.upgrade() else { return };
                    let imp = window.imp();
                    *imp.planner_from_coords.borrow_mut() = Some((feat.lat, feat.lon));
                    suppress_from.set(true);
                    imp.planner_from_entry.set_text(&feat.name);
                }
            },
        );

        // To entry: geocode → store planner_to_coords + name, dismiss keyboard
        let suppress_to = std::rc::Rc::new(std::cell::Cell::new(false));
        attach_geocoding_popover(
            self,
            &imp.planner_to_entry.get(),
            suppress_to.clone(),
            None, // no next field → dismiss keyboard
            {
                let window_weak = self.downgrade();
                move || {
                    window_weak
                        .upgrade()
                        .map(|w| {
                            let imp = w.imp();
                            (imp.map_profile.borrow().clone(), *imp.current_location.borrow())
                        })
                        .unwrap_or_else(|| ("online".to_string(), None))
                }
            },
            {
                let window_weak = self.downgrade();
                let suppress_to = suppress_to.clone();
                move |feat| {
                    let Some(window) = window_weak.upgrade() else { return };
                    let imp = window.imp();
                    *imp.planner_to_coords.borrow_mut() = Some((feat.lat, feat.lon));
                    *imp.planner_to_name.borrow_mut() = feat.name.clone();
                    suppress_to.set(true);
                    imp.planner_to_entry.set_text(&feat.name);

                    // Pan map to destination
                    let viewport = imp.map.viewport().expect("no viewport");
                    viewport.set_zoom_level(14.0);
                    viewport.set_location(feat.lat, feat.lon);
                }
            },
        );

        // "Get Directions" button
        imp.planner_go_button.connect_clicked({
            let window_weak = self.downgrade();
            move |_| {
                let Some(window) = window_weak.upgrade() else { return };
                let imp = window.imp();

                // Resolve origin: explicit From entry or current GPS location
                let origin = imp.planner_from_coords.borrow()
                    .or_else(|| *imp.current_location.borrow());

                let Some(origin) = origin else {
                    // Show a subtle indicator in the From field
                    imp.planner_from_entry.add_css_class("error");
                    glib::timeout_add_local_once(std::time::Duration::from_secs(2), {
                        let entry = imp.planner_from_entry.get();
                        move || { entry.remove_css_class("error"); }
                    });
                    return;
                };

                let Some(dest) = *imp.planner_to_coords.borrow() else {
                    imp.planner_to_entry.add_css_class("error");
                    glib::timeout_add_local_once(std::time::Duration::from_secs(2), {
                        let entry = imp.planner_to_entry.get();
                        move || { entry.remove_css_class("error"); }
                    });
                    return;
                };

                let dest_name = imp.planner_to_name.borrow().clone();

                // Close planner, show route preview
                imp.directions_button.set_active(false);

                window.request_route_preview_from(origin, dest, dest_name);
            }
        });
    }

    /// Fetch a route to `destination` using current GPS location as origin.
    fn request_route_preview(&self, destination: (f64, f64), dest_name: String) {
        let imp = self.imp();
        let origin = match *imp.current_location.borrow() {
            Some(loc) => loc,
            None => {
                glib::g_debug!("loci", "No location fix yet — press the location button first");
                return;
            }
        };
        self.request_route_preview_from(origin, destination, dest_name);
    }

    /// Fetch a route from `origin` to `destination`, draw it, and show the route preview panel.
    /// Works without GPS — origin can be any geocoded coordinate.
    fn request_route_preview_from(&self, origin: (f64, f64), destination: (f64, f64), dest_name: String) {
        let imp = self.imp();

        // Cache for mode-switch re-fetches
        *imp.preview_destination.borrow_mut() = Some(destination);
        *imp.preview_dest_name.borrow_mut() = dest_name.clone();
        *imp.pending_origin.borrow_mut() = Some(origin);

        // Show destination name and loading state immediately
        imp.route_dest_label.set_text(&dest_name);
        imp.route_distance_label.set_text("…");
        imp.route_time_label.set_text("");

        // Disable Start if no GPS — navigation requires a real position fix
        let has_gps = imp.current_location.borrow().is_some();
        imp.route_start_button.set_sensitive(has_gps);
        imp.route_start_button.set_tooltip_text(if has_gps {
            Some("Start Navigation")
        } else {
            Some("GPS required for navigation")
        });

        imp.route_preview_revealer.set_reveal_child(true);

        let profile = imp.current_profile.borrow().clone();
        let map_profile = imp.map_profile.borrow().clone();
        let (tx, rx) = std::sync::mpsc::channel::<ferrostar::models::Route>();
        let rx = Arc::new(Mutex::new(rx));

        std::thread::spawn(move || {
            match crate::routing::get_route_with_source(origin, destination, &map_profile, &profile) {
                Some(route) => { let _ = tx.send(route); }
                None => glib::g_warning!("loci", "Routing request failed"),
            }
        });

        glib::timeout_add_local(std::time::Duration::from_millis(200), {
            let window_weak = self.downgrade();
            move || {
                if let Ok(route) = rx.lock().unwrap().try_recv() {
                    let Some(window) = window_weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    let imp = window.imp();

                    // Draw the route geometry
                    let layer_opt = imp.route_layer.borrow().clone();
                    if let Some(layer) = layer_opt {
                        layer.remove_all();
                        for c in &route.geometry {
                            let node = shumate::Coordinate::new_full(c.lat, c.lng);
                            layer.add_node(&node);
                        }
                    }

                    // Compute totals from steps
                    let total_distance: f64 = route.steps.iter().map(|s| s.distance).sum();
                    let total_duration: f64 = route.steps.iter().map(|s| s.duration).sum();

                    imp.preview_total_distance.set(total_distance);
                    imp.preview_total_duration.set(total_duration);
                    Self::update_route_preview_labels(imp);

                    // Store for "Start" button
                    *imp.pending_route.borrow_mut() = Some(route);
                    *imp.pending_origin.borrow_mut() = Some(origin);

                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            }
        });
    }

    /// Start the Ferrostar NavigationController and begin streaming GPS updates.
    fn start_navigation(&self, origin: (f64, f64), route: ferrostar::models::Route) {
        use ferrostar::navigation_controller::NavigationController;
        use ferrostar::navigation_controller::Navigator;
        use ferrostar::navigation_controller::models::{
            NavigationControllerConfig, WaypointAdvanceMode, CourseFiltering,
        };
        use ferrostar::navigation_controller::step_advance::conditions::DistanceToEndOfStepCondition;
        use ferrostar::deviation_detection::RouteDeviationTracking;
        use ferrostar::models::{GeographicCoordinate, UserLocation};
        use std::sync::Arc;

        let step_advance = Arc::new(DistanceToEndOfStepCondition {
            distance: 25,
            minimum_horizontal_accuracy: 50,
        });
        let config = NavigationControllerConfig {
            waypoint_advance: WaypointAdvanceMode::WaypointWithinRange(50.0),
            step_advance_condition: step_advance.clone(),
            arrival_step_advance_condition: Arc::new(DistanceToEndOfStepCondition {
                distance: 10,
                minimum_horizontal_accuracy: 50,
            }),
            route_deviation_tracking: RouteDeviationTracking::Custom {
                detector: Arc::new(CurrentStepDeviationDetector { max_acceptable_deviation: 30.0 }),
            },
            snapped_location_course_filtering: CourseFiltering::SnapToRoute,
        };

        // Store destination for rerouting (last point in route geometry)
        let destination = route.geometry.last().map(|c| (c.lat, c.lng)).unwrap_or(origin);
        let imp = self.imp();
        *imp.nav_destination.borrow_mut() = Some(destination);
        imp.is_rerouting.set(false);
        *imp.last_reroute_at.borrow_mut() = None;
        *imp.reroute_result.lock().unwrap() = None;

        let controller = Arc::new(NavigationController::new(route, config));
        glib::g_debug!("loci", "[nav] NavigationController created, calling get_initial_state...");

        let initial_location = UserLocation {
            coordinates: GeographicCoordinate { lat: origin.0, lng: origin.1 },
            horizontal_accuracy: 10.0,
            course_over_ground: None,
            timestamp: std::time::SystemTime::now(),
            speed: None,
        };
        // get_initial_state() returns a NavState already in TripState::Navigating
        let first_state = controller.get_initial_state(initial_location);

        *imp.nav_controller.borrow_mut() = Some(controller.clone());

        // Store first state
        {
            let mut state_lock = imp.nav_state.lock().unwrap();
            *state_lock = Some(first_state.clone());
        }

        // Show nav banner and populate with first instruction
        imp.nav_banner_revealer.set_reveal_child(true);
        imp.place_revealer.set_reveal_child(false);
        Self::update_nav_banner(imp, &first_state);

        // Centre map on current position immediately
        if let Some(viewport) = imp.map.viewport() {
            viewport.set_zoom_level(16.0);
            viewport.set_location(origin.0, origin.1);
        }

        // Apply chase-camera 3D tilt
        imp.map.add_css_class("navigation-tilt");
        // Follow the user until they pan away; the location button re-centres.
        imp.following.set(true);
        imp.last_set_location.set(None);
        imp.last_set_zoom.set(None);
        imp.nav_zoom_target.set(None);
        imp.last_nav_fix_time.set(None);
        *imp.last_nav_pos.borrow_mut() = None;
        imp.recent_positions.borrow_mut().clear();
        // Start the render timer for smooth dot movement and the follow camera.
        *imp.extrap_fix0.borrow_mut() = None;
        *imp.extrap_fix1.borrow_mut() = None;
        self.install_render_timer();
        *imp.heading_from.borrow_mut() = None;
        *imp.heading_to.borrow_mut() = None;
        *imp.heading_anim_start.borrow_mut() = None;

        // Inhibit screen idle while navigating via GTK application inhibit API.
        if let Some(app) = self.application() {
            let cookie = app.inhibit(
                Some(self),
                gtk::ApplicationInhibitFlags::IDLE,
                Some("Turn-by-turn navigation is active"),
            );
            *imp.idle_inhibit.borrow_mut() = Some(cookie);
        }

        // Spawn GPS streaming thread — feeds location into the navigation controller
        let nav_state = imp.nav_state.clone();
        let (loc_tx, loc_rx) = std::sync::mpsc::channel::<(f64, f64, Option<f64>, Option<f64>)>();
        let loc_rx = Arc::new(Mutex::new(loc_rx));

        // GPS thread
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            rt.block_on(crate::location::stream_location(loc_tx));
        });

        // Navigation update loop — runs on main thread
        glib::timeout_add_local(std::time::Duration::from_millis(500), {
            let window_weak = self.downgrade();
            let nav_state = nav_state.clone();
            let loc_rx = loc_rx.clone();
            let reroute_result = imp.reroute_result.clone();
            move || {
                // Drain all pending location fixes (use the latest one)
                let mut latest: Option<(f64, f64, Option<f64>, Option<f64>)> = None;
                while let Ok(fix) = loc_rx.lock().unwrap().try_recv() {
                    latest = Some(fix);
                }

                let Some((lat, lon, gps_heading, gps_speed)) = latest else {
                    return glib::ControlFlow::Continue;
                };

                let Some(window) = window_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };

                use ferrostar::navigation_controller::Navigator;
                use ferrostar::models::{CourseOverGround, GeographicCoordinate, UserLocation};

                // Build course_over_ground from GPS heading, or compute from position delta
                // only when the device has moved enough to produce a meaningful bearing.
                // Tiny GPS jitter (~1-3m) between fixes produces garbage bearings, so we
                // require at least MIN_BEARING_DIST metres of movement before using the
                // computed fallback. If no reliable heading is available, pass None so
                // Ferrostar snaps position without spinning the map.
                const MIN_BEARING_DIST: f64 = 10.0; // metres
                let imp = window.imp();

                // Speed for the navigation zoom: prefer the GPS-reported value,
                // otherwise derive it from the previous fix and its timestamp.
                let fix_time = std::time::Instant::now();
                let computed_speed = match (*imp.last_nav_pos.borrow(), imp.last_nav_fix_time.get()) {
                    (Some((plat, plon)), Some(prev)) => {
                        let dt = fix_time.duration_since(prev).as_secs_f64();
                        (dt > 0.2).then(|| haversine(plat, plon, lat, lon) / dt)
                    }
                    _ => None,
                };
                imp.last_nav_fix_time.set(Some(fix_time));
                imp.nav_zoom_target.set(gps_speed.or(computed_speed).map(nav_zoom_for_speed));

                let course = gps_heading
                    .map(|h| {
                        glib::g_debug!("loci", "[nav] heading: GPS {h:.1}°");
                        CourseOverGround::new(h, None)
                    })
                    .or_else(|| {
                        let prev = imp.last_nav_pos.borrow().clone();
                        prev.and_then(|(prev_lat, prev_lon)| {
                            let dist = haversine(prev_lat, prev_lon, lat, lon);
                            if dist >= MIN_BEARING_DIST {
                                let bearing = compute_bearing(prev_lat, prev_lon, lat, lon);
                                glib::g_debug!("loci", "[nav] heading: computed {bearing:.1}° ({dist:.0}m delta)");
                                Some(CourseOverGround::new(bearing, None))
                            } else {
                                glib::g_debug!("loci", "[nav] heading: none (delta {dist:.1}m < threshold)");
                                None
                            }
                        })
                    });

                let user_location = UserLocation {
                    coordinates: GeographicCoordinate { lat, lng: lon },
                    horizontal_accuracy: 10.0,
                    course_over_ground: course,
                    timestamp: std::time::SystemTime::now(),
                    speed: None,
                };

                // If a reroute just completed, swap in the new controller and reset state.
                {
                    let mut slot = reroute_result.lock().unwrap();
                    if let Some(new_route) = slot.take() {
                        drop(slot);
                        let step_advance = Arc::new(DistanceToEndOfStepCondition {
                            distance: 25,
                            minimum_horizontal_accuracy: 50,
                        });
                        let new_config = NavigationControllerConfig {
                            waypoint_advance: WaypointAdvanceMode::WaypointWithinRange(50.0),
                            step_advance_condition: step_advance.clone(),
                            arrival_step_advance_condition: Arc::new(DistanceToEndOfStepCondition {
                                distance: 10,
                                minimum_horizontal_accuracy: 50,
                            }),
                            route_deviation_tracking: RouteDeviationTracking::Custom {
                                detector: Arc::new(CurrentStepDeviationDetector { max_acceptable_deviation: 30.0 }),
                            },
                            snapped_location_course_filtering: CourseFiltering::SnapToRoute,
                        };
                        let new_controller = Arc::new(NavigationController::new(new_route.clone(), new_config));
                        let first_state = new_controller.get_initial_state(UserLocation {
                            coordinates: GeographicCoordinate { lat, lng: lon },
                            horizontal_accuracy: 10.0,
                            course_over_ground: course,
                            timestamp: std::time::SystemTime::now(),
                            speed: None,
                        });
                        // Update route polyline on map
                        if let Some(layer) = imp.route_layer.borrow().clone() {
                            layer.remove_all();
                            for c in &new_route.geometry {
                                layer.add_node(&shumate::Coordinate::new_full(c.lat, c.lng));
                            }
                        }
                        *imp.nav_controller.borrow_mut() = Some(new_controller);
                        *nav_state.lock().unwrap() = Some(first_state.clone());
                        imp.is_rerouting.set(false);
                        Self::update_nav_banner(imp, &first_state);
                        return glib::ControlFlow::Continue;
                    }
                }

                // Read current controller (may have been swapped by reroute)
                let controller = imp.nav_controller.borrow().clone();
                let Some(controller) = controller else {
                    return glib::ControlFlow::Break;
                };

                // Update navigation state
                let new_state = {
                    let mut state_lock = nav_state.lock().unwrap();
                    let old_state = state_lock.take().unwrap();
                    let updated = controller.update_user_location(user_location, old_state);
                    *state_lock = Some(updated.clone());
                    updated
                };

                // Update location state
                *imp.current_location.borrow_mut() = Some((lat, lon));
                *imp.last_nav_pos.borrow_mut() = Some((lat, lon));

                // Animate the dot to the snapped position only when on-route.
                // When off-route, Ferrostar still snaps to the nearest point on the old
                // route line, so the dot would hang there instead of following the user.
                use ferrostar::navigation_controller::models::TripState;
                use ferrostar::deviation_detection::RouteDeviation;
                let (dot_pos, route_heading) = if let TripState::Navigating { snapped_user_location, deviation, .. } = new_state.trip_state() {
                    match deviation {
                        RouteDeviation::NoDeviation => {
                            let pos = (snapped_user_location.coordinates.lat, snapped_user_location.coordinates.lng);
                            // CourseFiltering::SnapToRoute gives us the route-tangent heading
                            // directly from Ferrostar — perfectly smooth, no GPS noise.
                            let heading = snapped_user_location.course_over_ground.map(|c| c.degrees as f64);
                            (pos, heading)
                        }
                        RouteDeviation::Deviation { .. } => ((lat, lon), None),
                    }
                } else {
                    ((lat, lon), None)
                };
                push_extrap_fix(&imp, dot_pos.0, dot_pos.1);

                // Map rotation heading.
                // On-route: use route-tangent from Ferrostar (zero noise, follows road curves).
                // Off-route or no route heading: fall back to GPS COG or extrap-fix bearing.
                let map_heading = route_heading.or_else(|| gps_heading).or_else(|| {
                    let f0 = *imp.extrap_fix0.borrow();
                    let f1 = *imp.extrap_fix1.borrow();
                    if let (Some((lat0, lon0, _)), Some((lat1, lon1, _))) = (f0, f1) {
                        let dist = haversine(lat0, lon0, lat1, lon1);
                        if dist >= MIN_BEARING_DIST {
                            Some(compute_bearing(lat0, lon0, lat1, lon1))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                });
                if let Some(heading) = map_heading {
                    let diff = imp.heading_to.borrow().map(|cur| {
                        let d = (heading - cur).abs();
                        if d > 180.0 { 360.0 - d } else { d }
                    }).unwrap_or(360.0);
                    // 5° dead-zone: suppress GPS jitter without hiding real turns
                    if diff >= 5.0 {
                        set_heading_target(&imp, heading);
                    }
                }

                // Update banner from trip state
                if matches!(new_state.trip_state(), TripState::Complete { .. }) {
                    imp.nav_instruction_label.set_text("You have arrived!");
                    imp.nav_distance_label.set_text("");
                    imp.nav_maneuver_icon.set_icon_name(Some("flag-filled-symbolic"));
                    glib::timeout_add_local_once(std::time::Duration::from_secs(3), {
                        let window_weak = window.downgrade();
                        move || {
                            if let Some(w) = window_weak.upgrade() {
                                w.stop_navigation();
                            }
                        }
                    });
                    return glib::ControlFlow::Break;
                }
                Self::update_nav_banner(imp, &new_state);

                // Trigger reroute when off-route (10 s cooldown, one request at a time)
                if let TripState::Navigating { deviation: RouteDeviation::Deviation { .. }, .. } = new_state.trip_state() {
                    let now = std::time::Instant::now();
                    let can_reroute = !imp.is_rerouting.get() && {
                        let last = imp.last_reroute_at.borrow();
                        last.map_or(true, |t| now.duration_since(t).as_secs() >= 10)
                    };
                    if can_reroute {
                        *imp.last_reroute_at.borrow_mut() = Some(now);
                        imp.is_rerouting.set(true);
                        imp.nav_instruction_label.set_text("Rerouting…");
                        imp.nav_distance_label.set_text("");
                        if let Some(dest) = *imp.nav_destination.borrow() {
                            let rr = reroute_result.clone();
                            let from = (lat, lon);
                            let profile = imp.current_profile.borrow().clone();
                            let map_profile = imp.map_profile.borrow().clone();
                            std::thread::spawn(move || {
                                match crate::routing::get_route_with_source(from, dest, &map_profile, &profile) {
                                    Some(route) => { *rr.lock().unwrap() = Some(route); }
                                    None => glib::g_warning!("loci", "[nav] reroute request failed"),
                                }
                            });
                        }
                    }
                }

                glib::ControlFlow::Continue
            }
        });
    }

    fn setup_navigation(&self) {
        let imp = self.imp();

        // Default profile
        *imp.current_profile.borrow_mut() = "car".to_string();

        // Mode toggle buttons: form a radio group so only one can be active at a time.
        imp.route_mode_bike_button.set_group(Some(&*imp.route_mode_car_button));
        imp.route_mode_foot_button.set_group(Some(&*imp.route_mode_car_button));

        let connect_mode_button = |btn: &gtk::ToggleButton, profile: &'static str| {
            btn.connect_toggled({
                let window_weak = self.downgrade();
                move |btn| {
                    if !btn.is_active() { return; }
                    let Some(window) = window_weak.upgrade() else { return };
                    let imp = window.imp();
                    *imp.current_profile.borrow_mut() = profile.to_string();
                    // Re-fetch the preview if it is currently visible
                    let dest = *imp.preview_destination.borrow();
                    let origin = {
                        let pending = *imp.pending_origin.borrow();
                        let current = *imp.current_location.borrow();
                        pending.or(current)
                    };
                    if imp.route_preview_revealer.reveals_child() {
                        if let (Some(origin), Some(dest)) = (origin, dest) {
                            let name = imp.preview_dest_name.borrow().clone();
                            window.request_route_preview_from(origin, dest, name);
                        }
                    }
                }
            });
        };
        connect_mode_button(&imp.route_mode_car_button, "car");
        connect_mode_button(&imp.route_mode_bike_button, "bike");
        connect_mode_button(&imp.route_mode_foot_button, "foot");

        imp.nav_stop_button.connect_clicked({
            let window_weak = self.downgrade();
            move |_| {
                if let Some(w) = window_weak.upgrade() {
                    w.stop_navigation();
                }
            }
        });
        imp.route_start_button.connect_clicked({
            let window_weak = self.downgrade();
            move |_| {
                let Some(window) = window_weak.upgrade() else { return };
                let imp = window.imp();
                let route = imp.pending_route.borrow_mut().take();
                let origin = imp.pending_origin.borrow_mut().take();
                if let (Some(route), Some(origin)) = (route, origin) {
                    imp.route_preview_revealer.set_reveal_child(false);
                    window.start_navigation(origin, route);
                }
            }
        });
        imp.route_cancel_button.connect_clicked({
            let window_weak = self.downgrade();
            move |_| {
                let Some(window) = window_weak.upgrade() else { return };
                let imp = window.imp();
                imp.route_preview_revealer.set_reveal_child(false);
                *imp.pending_route.borrow_mut() = None;
                *imp.pending_origin.borrow_mut() = None;
                *imp.preview_destination.borrow_mut() = None;
                // Clear drawn route
                let layer_opt = imp.route_layer.borrow().clone();
                if let Some(layer) = layer_opt { layer.remove_all(); }
                // Clear marker
                let marker_opt = imp.marker_layer.borrow().clone();
                if let Some(layer) = marker_opt { layer.remove_all(); }
            }
        });
    }

    fn stop_navigation(&self) {
        let imp = self.imp();
        imp.nav_banner_revealer.set_reveal_child(false);
        imp.nav_remaining_distance_label.set_text("");
        imp.nav_remaining_time_label.set_text("");
        *imp.nav_controller.borrow_mut() = None;
        *imp.nav_state.lock().unwrap() = None;
        *imp.pending_route.borrow_mut() = None;
        *imp.pending_origin.borrow_mut() = None;
        *imp.last_nav_pos.borrow_mut() = None;
        imp.following.set(true);
        imp.last_set_location.set(None);
        imp.last_set_zoom.set(None);
        imp.nav_zoom_target.set(None);
        imp.last_nav_fix_time.set(None);
        imp.recent_positions.borrow_mut().clear();
        *imp.heading_from.borrow_mut() = None;
        *imp.heading_to.borrow_mut() = None;
        *imp.heading_anim_start.borrow_mut() = None;
        *imp.nav_destination.borrow_mut() = None;
        *imp.last_reroute_at.borrow_mut() = None;
        imp.is_rerouting.set(false);
        *imp.reroute_result.lock().unwrap() = None;
        self.remove_render_timer();
        // Release screen idle inhibit
        if let Some(cookie) = imp.idle_inhibit.borrow_mut().take() {
            if let Some(app) = self.application() {
                app.uninhibit(cookie);
            }
        }
        imp.map.remove_css_class("navigation-tilt");
        if let Some(viewport) = imp.map.viewport() {
            viewport.set_rotation(0.0);
        }
        // Clear route layer and marker
        let layer_opt = imp.route_layer.borrow().clone();
        if let Some(layer) = layer_opt { layer.remove_all(); }
        if let Some(layer) = imp.marker_layer.borrow().as_ref() { layer.remove_all(); }
    }

    /// Update the nav banner labels/icon from a NavState.
    fn update_nav_banner(imp: &imp::LociWindow, state: &ferrostar::navigation_controller::models::NavState) {
        use ferrostar::navigation_controller::models::TripState;
        glib::g_debug!("loci", "[nav] update_nav_banner called, trip_state variant check...");
        match state.trip_state() {
            TripState::Navigating { visual_instruction, remaining_steps, progress, .. } => {
                glib::g_debug!("loci", "[nav] TripState::Navigating — dist_to_maneuver={:.0}m", progress.distance_to_next_maneuver);
                let text = visual_instruction
                    .as_ref()
                    .map(|vi| vi.primary_content.text.clone())
                    // visual_instruction is None when we're far from the trigger distance.
                    // Show the next step's instruction as a lookahead ("Turn right onto Oak Ave").
                    // remaining_steps[0] is the current step; [1] is the upcoming one.
                    .or_else(|| remaining_steps.get(1).map(|s| s.instruction.clone()))
                    .or_else(|| remaining_steps.first().and_then(|s| s.road_name.clone()))
                    .unwrap_or_else(|| "Continue".to_string());
                // When the visual_instruction hasn't triggered yet, peek at the current step's
                // (not-yet-active) visual instruction to get the correct upcoming maneuver icon.
                let lookahead_vi = remaining_steps.first()
                    .and_then(|s| s.visual_instructions.first());
                let icon = maneuver_icon(
                    visual_instruction.as_ref()
                        .and_then(|vi| vi.primary_content.maneuver_type)
                        .or_else(|| lookahead_vi.and_then(|vi| vi.primary_content.maneuver_type)),
                    visual_instruction.as_ref()
                        .and_then(|vi| vi.primary_content.maneuver_modifier)
                        .or_else(|| lookahead_vi.and_then(|vi| vi.primary_content.maneuver_modifier)),
                );
                imp.nav_instruction_label.set_text(&text);
                let units = imp.units.get();
                imp.nav_distance_label.set_text(&format_distance(progress.distance_to_next_maneuver, units));
                imp.nav_maneuver_icon.set_icon_name(Some(icon));
                imp.nav_remaining_distance_label.set_text(&format_distance(progress.distance_remaining, units));
                imp.nav_remaining_time_label.set_text(&format_duration(progress.duration_remaining));
            }
            TripState::Complete { .. } => glib::g_debug!("loci", "[nav] TripState::Complete"),
            TripState::Idle { .. } => glib::g_debug!("loci", "[nav] TripState::Idle — nav controller did not advance"),
        }
    }

    fn setup_location(&self) {
        let imp = self.imp();

        let (tx, rx) = std::sync::mpsc::channel::<(f64, f64, Option<f64>, Option<f64>)>();
        let rx = Arc::new(Mutex::new(rx));

        // Start streaming location continuously in the background.
        // This keeps the GeoClue session alive so GPS can improve over time.
        {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                rt.block_on(crate::location::stream_location(tx));
            });
        }

        // Location button: just re-centres the map on the latest known fix.
        // No longer starts a new session — the stream is already running.
        imp.location_button.connect_clicked({
            let window_weak = self.downgrade();
            move |_| {
                let Some(window) = window_weak.upgrade() else { return; };
                let imp = window.imp();
                let loc = imp.current_location.borrow().clone();
                if let Some((lat, lon)) = loc {
                    if let Some(viewport) = imp.map.viewport() {
                        if imp.nav_controller.borrow().is_some() {
                            // Re-lock the navigation camera and reset the zoom.
                            imp.following.set(true);
                            let zoom = imp.nav_zoom_target.get().unwrap_or(16.0);
                            viewport.set_zoom_level(zoom);
                            imp.last_set_zoom.set(Some(zoom));
                        } else {
                            viewport.set_zoom_level(15.0);
                            imp.last_set_zoom.set(None);
                        }
                        viewport.set_location(lat, lon);
                        imp.last_set_location.set(Some((lat, lon)));
                    }
                }
            }
        });

        // Poll for location fixes on the main loop and update the blue dot.
        // Only pan the map on the very first fix; after that the button does it.
        let first_fix_received = std::cell::Cell::new(false);
        glib::timeout_add_local(std::time::Duration::from_millis(500), {
            let window_weak = self.downgrade();
            move || {
                // Drain to latest fix only
                let mut latest = None;
                while let Ok(fix) = rx.lock().unwrap().try_recv() {
                    latest = Some(fix);
                }
                let Some((lat, lon, _heading, _speed)) = latest else {
                    return glib::ControlFlow::Continue;
                };
                let Some(window) = window_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let imp = window.imp();

                // Store fix so routing and location button can use it
                *imp.current_location.borrow_mut() = Some((lat, lon));

                // Pan to first fix automatically; subsequent pans only on button press
                if !first_fix_received.get() {
                    first_fix_received.set(true);
                    if let Some(viewport) = imp.map.viewport() {
                        viewport.set_zoom_level(15.0);
                        viewport.set_location(lat, lon);
                    }
                }

                // When not navigating the render timer is not running, so place
                // the dot straight from the fix; during navigation the timer
                // extrapolates it instead.
                if imp.nav_controller.borrow().is_none() {
                    if let Some(marker) = imp.location_marker.borrow().as_ref() {
                        marker.set_location(lat, lon);
                    }
                }
                glib::ControlFlow::Continue
            }
        });

    }

    /// Installs the 60 fps navigation render timer. It extrapolates the dot
    /// between GPS fixes and drives the follow camera; it only runs while
    /// navigating, so an idle app does not wake 60 times a second.
    fn install_render_timer(&self) {
        let imp = self.imp();
        if imp.render_timer.borrow().is_some() {
            return;
        }
        let id = glib::timeout_add_local(std::time::Duration::from_millis(16), {
            let window_weak = self.downgrade();
            move || {
                let Some(window) = window_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let imp = window.imp();

                // Compute display position via linear extrapolation (CoMaps-style).
                // We project the dot forward from the last GPS fix using the velocity
                // between the two most-recent fixes, so it continuously advances rather
                // than waiting for the next fix.  Safety limits mirror CoMaps:
                //   • interval between fixes must be 50 ms–2.1 s
                //   • extrapolation window capped at 2 s
                //   • maximum extrapolation distance 100 m (prevents runaway on bad data)
                let fix0 = *imp.extrap_fix0.borrow();
                let fix1 = *imp.extrap_fix1.borrow();

                let pos = if let Some((lat1, lon1, t1)) = fix1 {
                    if let Some((lat0, lon0, t0)) = fix0 {
                        let dt_between = (t1 - t0).as_secs_f64();
                        let dt_after   = t1.elapsed().as_secs_f64();
                        if dt_between > 0.05 && dt_between < 2.1 && dt_after < 2.0 {
                            let vel_lat = (lat1 - lat0) / dt_between;
                            let vel_lon = (lon1 - lon0) / dt_between;
                            let elat = lat1 + vel_lat * dt_after;
                            let elon = lon1 + vel_lon * dt_after;
                            if haversine(lat1, lon1, elat, elon) < 100.0 {
                                (elat, elon)
                            } else {
                                (lat1, lon1)
                            }
                        } else {
                            (lat1, lon1)
                        }
                    } else {
                        (lat1, lon1)
                    }
                } else {
                    return glib::ControlFlow::Continue;
                };
                let (lat, lon) = pos;

                if let Some(marker) = imp.location_marker.borrow().as_ref() {
                    marker.set_location(lat, lon);
                }

                // During navigation, keep the camera on the user unless they have
                // panned or zoomed away; the location button re-centres.
                if imp.nav_controller.borrow().is_some() {
                    if let Some(viewport) = imp.map.viewport() {
                        if imp.following.get() {
                            let moved = imp.last_set_location.get().is_some_and(|(llat, llon)| {
                                haversine(viewport.latitude(), viewport.longitude(), llat, llon) > 1.0
                            }) || imp.last_set_zoom.get().is_some_and(|lz| {
                                (viewport.zoom_level() - lz).abs() > 0.01
                            });
                            if moved {
                                imp.following.set(false);
                            }
                        }

                        if imp.following.get() {
                            viewport.set_location(lat, lon);
                            imp.last_set_location.set(Some((lat, lon)));

                            // Ease the zoom towards the speed-derived target.
                            if let Some(target) = imp.nav_zoom_target.get() {
                                let current = viewport.zoom_level();
                                let next = current + (target - current) * 0.12;
                                viewport.set_zoom_level(next);
                                imp.last_set_zoom.set(Some(next));
                            } else {
                                imp.last_set_zoom.set(Some(viewport.zoom_level()));
                            }

                            let h_start = *imp.heading_anim_start.borrow();
                            let h_from  = *imp.heading_from.borrow();
                            let h_to    = *imp.heading_to.borrow();
                            if let (Some(hs), Some(hf), Some(ht)) = (h_start, h_from, h_to) {
                                let h_dur = imp.heading_anim_dur.get().max(0.05);
                                let t_linear = (hs.elapsed().as_secs_f64() / h_dur).min(1.0);
                                // Smoothstep ease-in-out: t²(3−2t) — feels natural for map rotation
                                let ht_val = t_linear * t_linear * (3.0 - 2.0 * t_linear);
                                // Interpolate shortest angular path
                                let mut delta = ht - hf;
                                if delta > 180.0 { delta -= 360.0; }
                                if delta < -180.0 { delta += 360.0; }
                                let heading = ((hf + delta * ht_val) + 360.0) % 360.0;
                                viewport.set_rotation(-heading.to_radians());
                            }
                        }
                    }
                }
                glib::ControlFlow::Continue
            }
        });
        *imp.render_timer.borrow_mut() = Some(id);
    }

    /// Removes the navigation render timer, if one is installed.
    fn remove_render_timer(&self) {
        if let Some(id) = self.imp().render_timer.borrow_mut().take() {
            id.remove();
        }
    }

}

/// Target zoom for navigation: slower travel zooms in, faster travel zooms out.
/// Roughly 17 when stopped, easing down to 15 at highway speed.
fn nav_zoom_for_speed(meters_per_second: f64) -> f64 {
    let kmh = (meters_per_second * 3.6).max(0.0);
    (17.0 - kmh / 45.0).clamp(15.0, 17.0)
}

/// Maps a ferrostar maneuver type/modifier to the bundled symbolic icon name.
fn maneuver_icon(
    maneuver_type: Option<ferrostar::models::ManeuverType>,
    maneuver_modifier: Option<ferrostar::models::ManeuverModifier>,
) -> &'static str {
    use ferrostar::models::{ManeuverModifier as Mod, ManeuverType as Typ};
    match (maneuver_type, maneuver_modifier) {
        (Some(Typ::Arrive), _) => "flag-filled-symbolic",
        (Some(Typ::Depart), _) => "find-location-symbolic",
        (Some(Typ::Roundabout) | Some(Typ::Rotary) | Some(Typ::ExitRoundabout) | Some(Typ::ExitRotary), _) => "maps-direction-roundabout-symbolic",
        (_, Some(Mod::SharpLeft)) => "maps-direction-sharpleft-symbolic",
        (_, Some(Mod::SharpRight)) => "maps-direction-sharpright-symbolic",
        (_, Some(Mod::Left)) => "maps-direction-left-symbolic",
        (_, Some(Mod::Right)) => "maps-direction-right-symbolic",
        (_, Some(Mod::SlightLeft)) => "maps-direction-slightleft-symbolic",
        (_, Some(Mod::SlightRight)) => "maps-direction-slightright-symbolic",
        (_, Some(Mod::UTurn)) => "maps-direction-u-turn-right-symbolic",
        _ => "maps-direction-continue-symbolic",
    }
}

/// Attach a Photon geocoding autocomplete popover to any `GtkEntry`-like widget.
///
/// `on_select` is called on the main thread when the user taps a result row.
/// It receives a clone of the selected `PhotonFeature`.

/// Bridge trait so `attach_geocoding_popover` works with both `gtk::SearchEntry`
/// (activate from `SearchEntryExt`) and `gtk::Entry` (activate from `EntryExt`).
trait ConnectActivate {
    fn connect_activate_cb<F: Fn() + 'static>(&self, f: F);
}
impl ConnectActivate for gtk::SearchEntry {
    fn connect_activate_cb<F: Fn() + 'static>(&self, f: F) {
        self.connect_activate(move |_| f());
    }
}
impl ConnectActivate for gtk::Entry {
    fn connect_activate_cb<F: Fn() + 'static>(&self, f: F) {
        gtk::prelude::EntryExt::connect_activate(self, move |_| f());
    }
}

fn attach_geocoding_popover<E, F, P>(
    window: &LociWindow,
    entry: &E,
    suppress: std::rc::Rc<std::cell::Cell<bool>>,
    next_focus: Option<gtk::Widget>,
    get_profile: P,
    on_select: F,
)
where
    E: gtk::prelude::EditableExt
        + gtk::prelude::WidgetExt
        + ConnectActivate
        + glib::object::ObjectExt
        + Clone
        + 'static,
    F: Fn(PhotonFeature) + Clone + 'static,
    P: Fn() -> (String, Option<(f64, f64)>) + Clone + 'static,
{
    let list_box = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list_box.add_css_class("boxed-list");

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list_box)
        .max_content_height(320)
        .propagate_natural_height(true)
        .min_content_width(300)
        .build();

    let popover = gtk::Popover::builder()
        .child(&scrolled)
        .has_arrow(false)
        .position(gtk::PositionType::Bottom)
        .autohide(false)
        .can_focus(false)
        .build();
    popover.set_parent(entry);

    let results_store: std::rc::Rc<std::cell::RefCell<Vec<PhotonFeature>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));

    // Fills the popover with a list of places (search results or history).
    let render: std::rc::Rc<dyn Fn(Vec<PhotonFeature>)> = {
        let list_box = list_box.clone();
        let popover = popover.clone();
        let results_store = results_store.clone();
        std::rc::Rc::new(move |items: Vec<PhotonFeature>| {
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }
            *results_store.borrow_mut() = items.clone();
            if items.is_empty() {
                popover.popdown();
                return;
            }
            for feat in &items {
                let row = gtk::ListBoxRow::new();
                let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                hbox.set_margin_start(12);
                hbox.set_margin_end(12);
                hbox.set_margin_top(8);
                hbox.set_margin_bottom(8);

                let vbox = gtk::Box::new(gtk::Orientation::Vertical, 2);
                vbox.set_hexpand(true);

                let name_lbl = gtk::Label::builder()
                    .label(&feat.name)
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build();
                let sub_lbl = gtk::Label::builder()
                    .label(&feat.subtitle)
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build();
                sub_lbl.add_css_class("caption");
                sub_lbl.add_css_class("dim-label");

                vbox.append(&name_lbl);
                vbox.append(&sub_lbl);
                hbox.append(&vbox);
                row.set_child(Some(&hbox));
                list_box.append(&row);
            }
            popover.popup();
        })
    };

    // True while this entry genuinely has keyboard focus.
    // Managed exclusively by EventControllerFocus so it's never affected by
    // transient GTK-internal focus changes caused by popup().
    let entry_focused: std::rc::Rc<std::cell::Cell<bool>> =
        std::rc::Rc::new(std::cell::Cell::new(false));

    // True after a result is selected; prevents any in-flight result from
    // re-opening the popover even if focus hasn't left yet.
    let suppress_popup: std::rc::Rc<std::cell::Cell<bool>> =
        std::rc::Rc::new(std::cell::Cell::new(false));

    let (tx, rx) = std::sync::mpsc::channel::<Vec<PhotonFeature>>();
    let rx = Arc::new(Mutex::new(rx));

    // search_gen is declared early so the focus controller can increment it
    // on leave (cancelling any pending debounce timer).
    let search_gen: std::rc::Rc<std::cell::Cell<u64>> =
        std::rc::Rc::new(std::cell::Cell::new(0));

    // Track enter/leave explicitly.  On leave: dismiss popover AND increment
    // search_gen so any still-running debounce timer is cancelled – this
    // ensures results from a previous typing session never re-open the
    // popover when focus returns.
    {
        let focus_ctrl = gtk::EventControllerFocus::new();
        focus_ctrl.connect_enter({
            let ef = entry_focused.clone();
            let entry = entry.clone();
            let render = render.clone();
            let sp = suppress_popup.clone();
            move |_| {
                ef.set(true);
                // With an empty field, offer the recently used places.
                if entry.text().is_empty() {
                    sp.set(false);
                    render(crate::history::load());
                }
            }
        });
        focus_ctrl.connect_leave({
            let ef = entry_focused.clone();
            let popover = popover.clone();
            let sg = search_gen.clone();
            let sp = suppress_popup.clone();
            move |_| {
                ef.set(false);
                popover.popdown();
                // Invalidate any in-flight debounce so stale results are
                // discarded by the poll loop even after focus returns.
                sg.set(sg.get() + 1);
                // Also reset suppress_popup so the entry is ready next time.
                sp.set(false);
            }
        });
        entry.add_controller(focus_ctrl);
    }

    // Search on Enter
    entry.connect_activate_cb({
        let tx = tx.clone();
        let entry = entry.clone();
        let get_profile = get_profile.clone();
        move || {
            let query = entry.text().to_string();
            if query.is_empty() { return; }
            let tx = tx.clone();
            let (profile, near) = get_profile();
            std::thread::spawn(move || {
                let _ = tx.send(crate::geocoding::search_with_profile(&query, &profile, near));
            });
        }
    });

    // Debounced search-as-you-type
    entry.connect_changed({
        let tx = tx.clone();
        let popover = popover.clone();
        let suppress = suppress.clone();
        let suppress_popup = suppress_popup.clone();
        let search_gen = search_gen.clone();
        let entry = entry.clone();
        let render = render.clone();
        move |_| {
            // Ignore programmatic text changes (e.g. set_text after selection)
            if suppress.get() {
                suppress.set(false);
                return;
            }
            // User is typing again → allow popup again
            suppress_popup.set(false);
            let query = entry.text().to_string();
            if query.is_empty() {
                // Empty field → offer recent places again.
                render(crate::history::load());
                return;
            }
            if query.len() < 3 {
                popover.popdown();
                return;
            }
            let gen = search_gen.get() + 1;
            search_gen.set(gen);
            let tx = tx.clone();
            let sg = search_gen.clone();
            let get_profile = get_profile.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                if sg.get() != gen { return; }
                let tx = tx.clone();
                let (profile, near) = get_profile();
                std::thread::spawn(move || {
                    let _ = tx.send(crate::geocoding::search_with_profile(&query, &profile, near));
                });
            });
        }
    });

    // Poll loop: fill popover when results arrive
    glib::timeout_add_local(std::time::Duration::from_millis(100), {
        let render = render.clone();
        let suppress_popup = suppress_popup.clone();
        let entry_focused = entry_focused.clone();
        move || {
            if let Ok(results) = rx.lock().unwrap().try_recv() {
                // Only show if the entry currently has focus and no result
                // was recently selected.
                if suppress_popup.get() || !entry_focused.get() {
                    return glib::ControlFlow::Continue;
                }
                render(results);
            }
            glib::ControlFlow::Continue
        }
    });

    // On row tap: call the user's callback, then manage keyboard focus
    list_box.connect_row_activated({
        let popover = popover.clone();
        let suppress_popup = suppress_popup.clone();
        let results_store = results_store.clone();
        let entry = entry.clone();
        move |_, row| {
            let results = results_store.borrow();
            if let Some(feat) = results.get(row.index() as usize) {
                let feat = feat.clone();
                drop(results);
                suppress_popup.set(true); // block any in-flight search from re-opening
                popover.popdown();
                crate::history::record(&feat);
                on_select(feat);
                // Move focus: to next field (keeps keyboard open) or dismiss keyboard
                if let Some(w) = &next_focus {
                    w.grab_focus();
                } else {
                    // Clear window focus → virtual keyboard dismisses
                    if let Some(root) = entry.root() {
                        if let Ok(win) = root.downcast::<gtk::Window>() {
                            gtk::prelude::GtkWindowExt::set_focus(&win, None::<&gtk::Widget>);
                        }
                    }
                }
            }
        }
    });

    // Keep the popover alive by attaching it to the window
    let _ = window;
}

/// Push a new GPS fix into the two-slot linear-extrapolation buffer.
/// fix0 ← old fix1, fix1 ← (lat, lon, now).
fn push_extrap_fix(imp: &imp::LociWindow, lat: f64, lon: f64) {
    let now  = std::time::Instant::now();
    let prev = *imp.extrap_fix1.borrow();
    *imp.extrap_fix0.borrow_mut() = prev;
    *imp.extrap_fix1.borrow_mut() = Some((lat, lon, now));
}

/// Set a new heading animation target (degrees clockwise from north).
/// Duration = |delta_deg| / 60  → 60°/s rotation speed (90° takes 1.5 s).
/// Interpolation uses a smoothstep ease-in-out curve so turns accelerate
/// and decelerate naturally rather than spinning at constant speed.
fn set_heading_target(imp: &imp::LociWindow, heading: f64) {
    let dur = imp.heading_anim_dur.get().max(0.05);
    let current = {
        let hs = *imp.heading_anim_start.borrow();
        let hf = *imp.heading_from.borrow();
        let ht = *imp.heading_to.borrow();
        match (hs, hf, ht) {
            (Some(s), Some(f), Some(t)) => {
                let tval = (s.elapsed().as_secs_f64() / dur).min(1.0);
                let mut delta = t - f;
                if delta > 180.0 { delta -= 360.0; }
                if delta < -180.0 { delta += 360.0; }
                ((f + delta * tval) + 360.0) % 360.0
            }
            (_, _, Some(t)) => t,
            _ => heading,
        }
    };
    // Duration = delta_deg / 60  → ~60°/s rotation speed.
    // A 90° turn takes 1.5 s, a 10° correction takes 0.17 s.
    // Clamped to [0.15 s, 1.5 s] so small corrections finish quickly and
    // large U-turns don't spin forever.
    let mut delta = heading - current;
    if delta > 180.0  { delta -= 360.0; }
    if delta < -180.0 { delta += 360.0; }
    let new_dur = (delta.abs() / 60.0).clamp(0.15, 1.5);
    imp.heading_anim_dur.set(new_dur);
    *imp.heading_from.borrow_mut()       = Some(current);
    *imp.heading_to.borrow_mut()         = Some(heading);
    *imp.heading_anim_start.borrow_mut() = Some(std::time::Instant::now());
}
