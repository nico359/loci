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

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/nico359/loci/window.ui")]
    pub struct LociWindow {
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

        pub marker_layer: RefCell<Option<shumate::MarkerLayer>>,
        pub location_layer: RefCell<Option<shumate::MarkerLayer>>,
        pub route_layer: RefCell<Option<shumate::PathLayer>>,
        pub current_location: RefCell<Option<(f64, f64)>>,
        pub current_results: RefCell<Vec<PhotonFeature>>,

        // Pending route: stored after fetch, before user taps "Start"
        pub pending_route: RefCell<Option<ferrostar::models::Route>>,
        pub pending_origin: RefCell<Option<(f64, f64)>>,

        // Navigation state shared between the GPS thread and the main loop timer
        pub nav_controller: RefCell<Option<std::sync::Arc<ferrostar::navigation_controller::NavigationController>>>,
        pub nav_state: std::sync::Arc<std::sync::Mutex<Option<ferrostar::navigation_controller::models::NavState>>>,
        // Last known position used to compute bearing when GPS heading is unavailable
        pub last_nav_pos: RefCell<Option<(f64, f64)>>,
        // Smoothed heading in degrees (for stable map rotation)
        pub smoothed_heading: RefCell<Option<f64>>,
        // Screen idle inhibit — held during navigation, dropped to release
        pub idle_inhibit: RefCell<Option<ashpd::desktop::Request<()>>>,

        // Persistent location dot and animation state for smooth inter-fix movement
        pub location_marker: RefCell<Option<shumate::Marker>>,
        pub anim_from: RefCell<Option<(f64, f64)>>,
        pub anim_to: RefCell<Option<(f64, f64)>>,
        pub anim_start: RefCell<Option<std::time::Instant>>,
        // Heading animation (degrees) for smooth map rotation during navigation
        pub heading_from: RefCell<Option<f64>>,
        pub heading_to: RefCell<Option<f64>>,
        pub heading_anim_start: RefCell<Option<std::time::Instant>>,

        // Rerouting — destination stored so we can re-request a route when off-route
        pub nav_destination: RefCell<Option<(f64, f64)>>,
        pub last_reroute_at: RefCell<Option<std::time::Instant>>,
        pub is_rerouting: std::cell::Cell<bool>,
        pub reroute_result: std::sync::Arc<std::sync::Mutex<Option<ferrostar::models::Route>>>,
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
            self.obj().setup_map();
            self.obj().setup_geocoding();
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
            eprintln!("ERROR: libshumate was compiled without vector tile support");
            return;
        }

        // Use the same tile server as GNOME Maps — static URL, OpenMapTiles v3 schema.
        // Glyphs from James Westman's font server (same as GNOME Maps).
        let style = serde_json::json!({
            "version": 8,
            "name": "Loci",
            "sources": {
                "vector-tiles": {
                    "type": "vector",
                    "tiles": ["https://tileserver.gnome.org/data/v3/{z}/{x}/{y}.pbf"],
                    "minzoom": 0,
                    "maxzoom": 14
                }
            },
            "glyphs": "https://tiles.maps.jwestman.net/fonts/{fontstack}/{range}.pbf",
            "layers": [
                {"id": "background", "type": "background",
                 "paint": {"background-color": "#f0ebe3"}},
                {"id": "water", "type": "fill",
                 "source": "vector-tiles", "source-layer": "water",
                 "paint": {"fill-color": "#a8d4f0"}},
                {"id": "landcover-park", "type": "fill",
                 "source": "vector-tiles", "source-layer": "landcover",
                 "filter": ["in", "class", "grass", "park", "forest"],
                 "paint": {"fill-color": "#c8e6c0"}},
                {"id": "road-minor", "type": "line",
                 "source": "vector-tiles", "source-layer": "transportation",
                 "filter": ["in", "class", "minor", "path", "service"],
                 "paint": {"line-color": "#d8d0c8", "line-width": 1}},
                {"id": "road-secondary", "type": "line",
                 "source": "vector-tiles", "source-layer": "transportation",
                 "filter": ["in", "class", "secondary", "tertiary"],
                 "paint": {"line-color": "#c0b8b0", "line-width": 2}},
                {"id": "road-primary", "type": "line",
                 "source": "vector-tiles", "source-layer": "transportation",
                 "filter": ["in", "class", "primary", "trunk", "motorway"],
                 "paint": {"line-color": "#e8c070", "line-width": 3}},
                {"id": "building", "type": "fill",
                 "source": "vector-tiles", "source-layer": "building",
                 "paint": {"fill-color": "#dbd5cc", "fill-outline-color": "#c8c0b8"}},
                {"id": "place-label", "type": "symbol",
                 "source": "vector-tiles", "source-layer": "place",
                 "layout": {"text-field": ["get", "name:latin"], "text-size": 13},
                 "paint": {"text-color": "#333", "text-halo-color": "#fff",
                           "text-halo-width": 1}}
            ]
        });

        match shumate::VectorRenderer::new("gnome-tiles", &style.to_string()) {
            Ok(renderer) => {
                imp.map.set_map_source(Some(&renderer));
                let viewport = imp.map.viewport().expect("SimpleMap has no viewport");
                viewport.set_zoom_level(12.0);
                viewport.set_location(52.5200, 13.4050);

                // Marker layers: one for search pins, one for current location
                let marker_layer = shumate::MarkerLayer::new(&viewport);
                imp.map.add_overlay_layer(&marker_layer);
                *imp.marker_layer.borrow_mut() = Some(marker_layer);

                let location_layer = shumate::MarkerLayer::new(&viewport);
                imp.map.add_overlay_layer(&location_layer);

                // Create the persistent location dot once; we animate it rather than recreate it.
                let location_marker = shumate::Marker::new();
                let dot = gtk::Box::builder().width_request(18).height_request(18).build();
                dot.add_css_class("location-dot");
                location_marker.set_child(Some(&dot));
                location_layer.add_marker(&location_marker);
                *imp.location_marker.borrow_mut() = Some(location_marker);

                *imp.location_layer.borrow_mut() = Some(location_layer);

                // Path layer for route geometry (drawn below the marker layers)
                let route_layer = shumate::PathLayer::new(&viewport);
                route_layer.set_stroke_width(5.0);
                route_layer.set_stroke_color(Some(&gdk::RGBA::new(0.2, 0.5, 1.0, 0.9)));
                imp.map.add_overlay_layer(&route_layer);
                *imp.route_layer.borrow_mut() = Some(route_layer);
            }
            Err(e) => eprintln!("VectorRenderer::new error: {e}"),
        }

        // Zoom buttons
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
    }

    fn setup_geocoding(&self) {
        let imp = self.imp();

        // Results list inside a popover anchored to the search entry
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
            .autohide(true)
            .build();
        popover.set_parent(&imp.search_entry.get());

        // Channel: background search threads → main thread results
        let (tx, rx) = std::sync::mpsc::channel::<Vec<PhotonFeature>>();
        let rx = Arc::new(Mutex::new(rx));

        // On Enter or after a short typing pause: spawn search thread
        imp.search_entry.connect_activate({
            let tx = tx.clone();
            move |entry| {
                let query = entry.text().to_string();
                if query.is_empty() { return; }
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let results = crate::geocoding::search(&query);
                    let _ = tx.send(results);
                });
            }
        });

        // Search-as-you-type: debounce 400ms so we don't hammer the API on every keystroke.
        // Generation counter approach: bump gen on each keystroke; only the latest timer fires.
        let search_gen: std::rc::Rc<std::cell::Cell<u64>> =
            std::rc::Rc::new(std::cell::Cell::new(0));
        imp.search_entry.connect_changed({
            let tx = tx.clone();
            let search_gen = search_gen.clone();
            move |entry| {
                let query = entry.text().to_string();
                if query.len() < 3 { return; }
                let gen = search_gen.get() + 1;
                search_gen.set(gen);
                let tx = tx.clone();
                let search_gen2 = search_gen.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                    if search_gen2.get() != gen { return; }
                    std::thread::spawn(move || {
                        let results = crate::geocoding::search(&query);
                        let _ = tx.send(results);
                    });
                });
            }
        });

        // Poll for results on the main loop; update popover when they arrive
        glib::timeout_add_local(std::time::Duration::from_millis(100), {
            let rx = rx.clone();
            let list_box = list_box.clone();
            let popover = popover.clone();
            let window_weak = self.downgrade();
            move || {
                if let Ok(results) = rx.lock().unwrap().try_recv() {
                    // Clear old rows
                    while let Some(child) = list_box.first_child() {
                        list_box.remove(&child);
                    }

                    if let Some(window) = window_weak.upgrade() {
                        *window.imp().current_results.borrow_mut() = results.clone();
                    }

                    if results.is_empty() {
                        popover.popdown();
                    } else {
                        for feat in &results {
                            let row = gtk::ListBoxRow::new();
                            let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                            hbox.set_margin_start(12);
                            hbox.set_margin_end(12);
                            hbox.set_margin_top(8);
                            hbox.set_margin_bottom(8);

                            let vbox = gtk::Box::new(gtk::Orientation::Vertical, 2);
                            vbox.set_hexpand(true);

                            let name = gtk::Label::builder()
                                .label(&feat.name)
                                .xalign(0.0)
                                .ellipsize(gtk::pango::EllipsizeMode::End)
                                .build();

                            let subtitle = gtk::Label::builder()
                                .label(&feat.subtitle)
                                .xalign(0.0)
                                .ellipsize(gtk::pango::EllipsizeMode::End)
                                .build();
                            subtitle.add_css_class("caption");
                            subtitle.add_css_class("dim-label");

                            vbox.append(&name);
                            vbox.append(&subtitle);

                            hbox.append(&vbox);
                            row.set_child(Some(&hbox));
                            list_box.append(&row);
                        }
                        popover.popup();
                    }
                }
                glib::ControlFlow::Continue
            }
        });

        // On row tap: pan map, place marker, and fetch route preview
        list_box.connect_row_activated({
            let popover = popover.clone();
            let window_weak = self.downgrade();
            move |_, row| {
                let Some(window) = window_weak.upgrade() else { return };
                let imp = window.imp();
                let results = imp.current_results.borrow();
                let Some(feat) = results.get(row.index() as usize) else { return };
                let (lat, lon) = (feat.lat, feat.lon);
                let dest_name = feat.name.clone();
                drop(results);

                // Pan map
                let viewport = imp.map.viewport().expect("no viewport");
                viewport.set_zoom_level(14.0);
                viewport.set_location(lat, lon);

                // Replace marker
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

                popover.popdown();

                // Fetch route and show preview panel
                window.request_route_preview((lat, lon), dest_name);
            }
        });
    }

    /// Fetch a route to `destination`, draw it on the map, then show the route preview panel.
    fn request_route_preview(&self, destination: (f64, f64), dest_name: String) {
        let imp = self.imp();
        let origin = match *imp.current_location.borrow() {
            Some(loc) => loc,
            None => {
                eprintln!("No location fix yet — press the location button first");
                return;
            }
        };

        // Show destination name immediately; distance/time fill in when route arrives
        imp.route_dest_label.set_text(&dest_name);
        imp.route_distance_label.set_text("…");
        imp.route_time_label.set_text("");
        imp.route_preview_revealer.set_reveal_child(true);

        let (tx, rx) = std::sync::mpsc::channel::<ferrostar::models::Route>();
        let rx = Arc::new(Mutex::new(rx));

        std::thread::spawn(move || {
            match crate::routing::get_route(origin, destination, crate::routing::DEFAULT_VALHALLA_URL, "auto") {
                Some(route) => { let _ = tx.send(route); }
                None => eprintln!("Routing request failed"),
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

                    imp.route_distance_label.set_text(&format_distance(total_distance));
                    imp.route_time_label.set_text(&format_duration(total_duration));

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
            route_deviation_tracking: RouteDeviationTracking::StaticThreshold {
                minimum_horizontal_accuracy: 25,
                max_acceptable_deviation: 25.0,
            },
            snapped_location_course_filtering: CourseFiltering::Raw,
        };

        // Store destination for rerouting (last point in route geometry)
        let destination = route.geometry.last().map(|c| (c.lat, c.lng)).unwrap_or(origin);
        let imp = self.imp();
        *imp.nav_destination.borrow_mut() = Some(destination);
        imp.is_rerouting.set(false);
        *imp.last_reroute_at.borrow_mut() = None;
        *imp.reroute_result.lock().unwrap() = None;

        let controller = Arc::new(NavigationController::new(route, config));
        eprintln!("[nav] NavigationController created, calling get_initial_state...");

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
        Self::update_nav_banner(imp, &first_state);

        // Centre map on current position immediately
        if let Some(viewport) = imp.map.viewport() {
            viewport.set_zoom_level(17.0);
            viewport.set_location(origin.0, origin.1);
        }

        // Apply chase-camera 3D tilt
        imp.map.add_css_class("navigation-tilt");
        *imp.last_nav_pos.borrow_mut() = None;
        *imp.smoothed_heading.borrow_mut() = None;
        *imp.heading_from.borrow_mut() = None;
        *imp.heading_to.borrow_mut() = None;
        *imp.heading_anim_start.borrow_mut() = None;

        // Inhibit screen idle while navigating
        {
            let window_weak = self.downgrade();
            glib::spawn_future_local(async move {
                match ashpd::desktop::inhibit::InhibitProxy::new().await {
                    Ok(proxy) => {
                        use ashpd::desktop::inhibit::{InhibitFlags, InhibitOptions};
                        use ashpd::enumflags2::BitFlags;
                        let flags = BitFlags::from(InhibitFlags::Idle);
                        let opts = InhibitOptions::default().set_reason(Some("Turn-by-turn navigation is active"));
                        match proxy.inhibit(None, flags, opts).await {
                            Ok(req) => {
                                if let Some(w) = window_weak.upgrade() {
                                    *w.imp().idle_inhibit.borrow_mut() = Some(req);
                                }
                            }
                            Err(e) => eprintln!("[inhibit] inhibit() failed: {e}"),
                        }
                    }
                    Err(e) => eprintln!("[inhibit] InhibitProxy::new() failed: {e}"),
                }
            });
        }

        // Spawn GPS streaming thread — feeds location into the navigation controller
        let nav_state = imp.nav_state.clone();
        let (loc_tx, loc_rx) = std::sync::mpsc::channel::<(f64, f64, Option<f64>)>();
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
                let mut latest: Option<(f64, f64, Option<f64>)> = None;
                while let Ok(fix) = loc_rx.lock().unwrap().try_recv() {
                    latest = Some(fix);
                }

                let Some((lat, lon, gps_heading)) = latest else {
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
                let course = gps_heading
                    .map(|h| {
                        eprintln!("[nav] heading: GPS {h:.1}°");
                        CourseOverGround::new(h, None)
                    })
                    .or_else(|| {
                        let prev = imp.last_nav_pos.borrow().clone();
                        prev.and_then(|(prev_lat, prev_lon)| {
                            let dist = haversine_m(prev_lat, prev_lon, lat, lon);
                            if dist >= MIN_BEARING_DIST {
                                let bearing = compute_bearing(prev_lat, prev_lon, lat, lon);
                                eprintln!("[nav] heading: computed {bearing:.1}° ({dist:.0}m delta)");
                                Some(CourseOverGround::new(bearing, None))
                            } else {
                                eprintln!("[nav] heading: none (delta {dist:.1}m < threshold)");
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
                            route_deviation_tracking: RouteDeviationTracking::StaticThreshold {
                                minimum_horizontal_accuracy: 25,
                                max_acceptable_deviation: 25.0,
                            },
                            snapped_location_course_filtering: CourseFiltering::Raw,
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

                // Advance animation — the 60fps timer handles marker + viewport movement
                set_anim_target(&imp, lat, lon);

                // Update heading animation target so the 60fps timer rotates the viewport smoothly.
                // Only update when we actually got a fresh reliable heading this fix.
                if let Some(ref cog) = course {
                    set_heading_target(&imp, cog.degrees as f64);
                }

                // Update banner from trip state
                use ferrostar::navigation_controller::models::TripState;
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
                use ferrostar::deviation_detection::RouteDeviation;
                if let TripState::Navigating { deviation: RouteDeviation::OffRoute { .. }, .. } = new_state.trip_state() {
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
                            std::thread::spawn(move || {
                                match crate::routing::get_route(from, dest, crate::routing::DEFAULT_VALHALLA_URL, "auto") {
                                    Some(route) => { *rr.lock().unwrap() = Some(route); }
                                    None => eprintln!("[nav] reroute request failed"),
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
        *imp.nav_controller.borrow_mut() = None;
        *imp.nav_state.lock().unwrap() = None;
        *imp.pending_route.borrow_mut() = None;
        *imp.pending_origin.borrow_mut() = None;
        *imp.last_nav_pos.borrow_mut() = None;
        *imp.smoothed_heading.borrow_mut() = None;
        *imp.heading_from.borrow_mut() = None;
        *imp.heading_to.borrow_mut() = None;
        *imp.heading_anim_start.borrow_mut() = None;
        *imp.nav_destination.borrow_mut() = None;
        *imp.last_reroute_at.borrow_mut() = None;
        imp.is_rerouting.set(false);
        *imp.reroute_result.lock().unwrap() = None;
        // Release screen idle inhibit
        *imp.idle_inhibit.borrow_mut() = None;
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
        eprintln!("[nav] update_nav_banner called, trip_state variant check...");
        match state.trip_state() {
            TripState::Navigating { visual_instruction, remaining_steps, progress, .. } => {
                eprintln!("[nav] TripState::Navigating — dist_to_maneuver={:.0}m", progress.distance_to_next_maneuver);
                let text = visual_instruction
                    .as_ref()
                    .map(|vi| vi.primary_content.text.as_str())
                    .or_else(|| remaining_steps.first().and_then(|s| s.road_name.as_deref()))
                    .unwrap_or("Continue")
                    .to_string();
                let icon = maneuver_icon(
                    visual_instruction.as_ref().and_then(|vi| vi.primary_content.maneuver_type),
                    visual_instruction.as_ref().and_then(|vi| vi.primary_content.maneuver_modifier),
                );
                imp.nav_instruction_label.set_text(&text);
                imp.nav_distance_label.set_text(&format_distance(progress.distance_to_next_maneuver));
                imp.nav_maneuver_icon.set_icon_name(Some(icon));
            }
            TripState::Complete { .. } => eprintln!("[nav] TripState::Complete"),
            TripState::Idle { .. } => eprintln!("[nav] TripState::Idle — nav controller did not advance"),
        }
    }

    fn setup_location(&self) {
        let imp = self.imp();

        let (tx, rx) = std::sync::mpsc::channel::<(f64, f64, Option<f64>)>();
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
                        viewport.set_zoom_level(15.0);
                        viewport.set_location(lat, lon);
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
                let Some((lat, lon, _heading)) = latest else {
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

                // Advance animation toward this new GPS fix
                set_anim_target(&imp, lat, lon);
                glib::ControlFlow::Continue
            }
        });

        // 60 fps animation timer — smoothly interpolates the location dot (and viewport
        // during navigation) between GPS fixes so movement looks fluid instead of jumping.
        glib::timeout_add_local(std::time::Duration::from_millis(16), {
            let window_weak = self.downgrade();
            move || {
                let Some(window) = window_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let imp = window.imp();

                let anim_start = *imp.anim_start.borrow();
                let anim_from  = *imp.anim_from.borrow();
                let anim_to    = *imp.anim_to.borrow();

                if let (Some(start), Some(from), Some(to)) = (anim_start, anim_from, anim_to) {
                    let t = (start.elapsed().as_secs_f64() / 1.0_f64).min(1.0);
                    let lat = from.0 + (to.0 - from.0) * t;
                    let lon = from.1 + (to.1 - from.1) * t;

                    if let Some(marker) = imp.location_marker.borrow().as_ref() {
                        marker.set_location(lat, lon);
                    }

                    // During navigation also pan the viewport and apply animated heading
                    if imp.nav_controller.borrow().is_some() {
                        if let Some(viewport) = imp.map.viewport() {
                            viewport.set_location(lat, lon);

                            let h_start = *imp.heading_anim_start.borrow();
                            let h_from  = *imp.heading_from.borrow();
                            let h_to    = *imp.heading_to.borrow();
                            if let (Some(hs), Some(hf), Some(ht)) = (h_start, h_from, h_to) {
                                let ht_val = (hs.elapsed().as_secs_f64() / 1.0_f64).min(1.0);
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
    }
}

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

fn format_distance(meters: f64) -> String {
    if meters >= 1000.0 {
        format!("{:.1} km", meters / 1000.0)
    } else {
        format!("{} m", meters.round() as u32)
    }
}

fn format_duration(seconds: f64) -> String {
    let total = seconds.round() as u64;
    let hours = total / 3600;
    let mins = (total % 3600) / 60;
    if hours > 0 {
        format!("{} h {} min", hours, mins)
    } else if mins == 0 {
        "< 1 min".to_string()
    } else {
        format!("{} min", mins)
    }
}

/// Haversine distance in metres between two WGS-84 coordinates.
fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

/// Great-circle bearing from point 1 → point 2, in clockwise degrees from north [0, 360).
fn compute_bearing(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let lat1r = lat1.to_radians();
    let lat2r = lat2.to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let y = dlon.sin() * lat2r.cos();
    let x = lat1r.cos() * lat2r.sin() - lat1r.sin() * lat2r.cos() * dlon.cos();
    let bearing = y.atan2(x).to_degrees();
    (bearing + 360.0) % 360.0
}

/// Exponential smoothing for an angle in degrees, handling the 0/360 wrap-around.
fn smooth_angle(prev: f64, target: f64, alpha: f64) -> f64 {
    // Compute shortest angular delta
    let mut delta = target - prev;
    if delta > 180.0 { delta -= 360.0; }
    if delta < -180.0 { delta += 360.0; }
    let result = prev + alpha * delta;
    (result + 360.0) % 360.0
}

/// Update the animation target for the location dot.
/// Takes the current interpolated position as `anim_from` so transitions are seamless
/// even if the previous animation hadn't completed yet.
fn set_anim_target(imp: &imp::LociWindow, lat: f64, lon: f64) {
    let from = {
        let start = *imp.anim_start.borrow();
        let from  = *imp.anim_from.borrow();
        let to    = *imp.anim_to.borrow();
        match (start, from, to) {
            (Some(s), Some(f), Some(t)) => {
                let elapsed = s.elapsed().as_secs_f64();
                let tval = (elapsed / 1.0_f64).min(1.0);
                (f.0 + (t.0 - f.0) * tval, f.1 + (t.1 - f.1) * tval)
            }
            (_, _, Some(t)) => t,
            _ => (lat, lon),
        }
    };
    *imp.anim_from.borrow_mut()  = Some(from);
    *imp.anim_to.borrow_mut()    = Some((lat, lon));
    *imp.anim_start.borrow_mut() = Some(std::time::Instant::now());
}

/// Set a new heading animation target (degrees clockwise from north).
/// Captures the current interpolated angle as `heading_from` for seamless transitions.
fn set_heading_target(imp: &imp::LociWindow, heading: f64) {
    let current = {
        let hs = *imp.heading_anim_start.borrow();
        let hf = *imp.heading_from.borrow();
        let ht = *imp.heading_to.borrow();
        match (hs, hf, ht) {
            (Some(s), Some(f), Some(t)) => {
                let tval = (s.elapsed().as_secs_f64() / 1.0_f64).min(1.0);
                let mut delta = t - f;
                if delta > 180.0 { delta -= 360.0; }
                if delta < -180.0 { delta += 360.0; }
                ((f + delta * tval) + 360.0) % 360.0
            }
            (_, _, Some(t)) => t,
            _ => heading,
        }
    };
    *imp.heading_from.borrow_mut()       = Some(current);
    *imp.heading_to.borrow_mut()         = Some(heading);
    *imp.heading_anim_start.borrow_mut() = Some(std::time::Instant::now());
}
