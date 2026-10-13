#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod capture;
#[cfg(windows)]
mod hdr;
// Only the Windows HDR window lays screens out outside Slint (tests use it everywhere).
#[cfg_attr(not(windows), allow(dead_code))]
mod layout;
mod presets;
mod settings;
mod view;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use slint::{CloseRequestResponse, ComponentHandle, Image, ModelRc, SharedString, VecModel};

use audio::AudioOutput;
use capture::frame::Frame;
use capture::{CaptureManager, DeviceInfo, Event};
use settings::{NUM_PROFILES, Settings};

slint::include_modules!();

struct App {
    ui: AppWindow,
    /// Second window for split mode. Not created in kiosk mode, where the
    /// display backend drives a single fullscreen output.
    bottom: Option<BottomWindow>,
    kiosk: bool,
    /// Whether the HOME Menu (or a page) is showing over the capture.
    menu_shown: Cell<bool>,
    #[cfg(windows)]
    hdr: RefCell<Option<hdr::HdrOutput>>,
    #[cfg(windows)]
    hdr_visible: Cell<bool>,
    capture: CaptureManager,
    audio: RefCell<AudioOutput>,
    settings: RefCell<Settings>,
    devices: RefCell<Vec<DeviceInfo>>,
    connected: RefCell<Option<DeviceInfo>>,
    last_frame: RefCell<Option<Frame>>,
    frame_count: Cell<i32>,
    audio_devices: RefCell<Vec<String>>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

fn with_app(f: impl FnOnce(&Rc<App>)) {
    APP.with(|app| {
        if let Some(app) = app.borrow().as_ref() {
            f(app);
        }
    });
}

fn string_model(items: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(items.into_iter().map(SharedString::from).collect::<Vec<_>>()))
}

impl App {
    fn is_3ds(&self) -> bool {
        self.connected.borrow().as_ref().is_none_or(|d| d.board.is_3ds())
    }

    /// Copy settings into the UI properties.
    fn push_settings_to_ui(&self) {
        let s = self.settings.borrow();
        let ui = &self.ui;
        ui.set_layout(s.layout.clamp(0, 5));
        ui.set_bottom_scale(s.bottom_scale);
        ui.set_screen_offset(s.screen_offset);
        ui.set_separator(s.separator as f32);
        ui.set_integer_scaling(s.integer_scaling);
        ui.set_smoothing(s.smoothing);
        ui.set_top_rotation(s.top_rotation);
        ui.set_bottom_rotation(s.bottom_rotation);
        ui.set_game_crops(s.game_crops);
        ui.set_volume(s.volume as f32);
        ui.set_muted(s.muted);
        ui.set_enable_3d(s.enable_3d);
        ui.set_mode_3d(s.mode_3d.clamp(0, 1));
        ui.set_split_windows(s.split_windows);
        ui.set_auto_connect(s.auto_connect);
        ui.set_theme(s.theme);
        ui.set_filter_style(s.filter_style.clamp(0, 3));
        ui.set_filter_strength(s.filter_strength.clamp(0.0, 100.0));
        ui.set_hdr_output(s.hdr_output);
        ui.set_hdr_brightness(s.hdr_brightness.clamp(100.0, 1000.0));
        ui.global::<Theme>().set_index(s.theme);
        let devices = self.audio_devices.borrow();
        let index = devices.iter().position(|d| *d == s.audio_device).unwrap_or(0);
        ui.set_audio_device_index(index as i32);
        drop(s);
        self.refresh_crops();
    }

    /// Read the UI properties back into settings.
    fn pull_settings_from_ui(&self) {
        let ui = &self.ui;
        let mut s = self.settings.borrow_mut();
        s.layout = ui.get_layout();
        s.bottom_scale = ui.get_bottom_scale();
        s.screen_offset = ui.get_screen_offset();
        s.separator = ui.get_separator() as i32;
        s.integer_scaling = ui.get_integer_scaling();
        s.smoothing = ui.get_smoothing();
        s.top_rotation = ui.get_top_rotation();
        s.bottom_rotation = ui.get_bottom_rotation();
        s.game_crops = ui.get_game_crops();
        if self.is_3ds() {
            s.crop_3ds = ui.get_crop_index();
        } else {
            s.crop_ds = ui.get_crop_index();
        }
        s.volume = ui.get_volume().round() as i32;
        s.muted = ui.get_muted();
        s.enable_3d = ui.get_enable_3d();
        s.mode_3d = ui.get_mode_3d();
        s.split_windows = ui.get_split_windows();
        s.auto_connect = ui.get_auto_connect();
        s.theme = ui.get_theme();
        s.filter_style = ui.get_filter_style();
        s.filter_strength = ui.get_filter_strength();
        s.hdr_output = ui.get_hdr_output();
        s.hdr_brightness = ui.get_hdr_brightness();
        let index = ui.get_audio_device_index().max(0) as usize;
        s.audio_device = if index == 0 { String::new() } else { self.audio_devices.borrow().get(index).cloned().unwrap_or_default() };
    }

    fn refresh_crops(&self) {
        let s = self.settings.borrow();
        let is_3ds = self.is_3ds();
        let names: Vec<String> = view::crops(is_3ds, s.game_crops).iter().map(|c| c.name.to_string()).collect();
        let saved = if is_3ds { s.crop_3ds } else { s.crop_ds };
        let index = saved.clamp(0, names.len() as i32 - 1);
        self.ui.set_crop_names(string_model(names));
        self.ui.set_crop_index(index);
    }

    /// Apply settings that need more than a property change.
    fn apply_settings(&self, previous: &Settings) {
        let s = self.settings.borrow().clone();
        if let Err(e) = s.save() {
            self.toast(&format!("Could not save settings: {e}"));
        }
        {
            let audio = self.audio.borrow();
            audio.settings.set_volume(s.volume as f32 / 100.0);
            audio.settings.set_muted(s.muted);
        }
        if s.audio_device != previous.audio_device {
            self.open_audio();
        }
        self.capture.set_3d(s.enable_3d);
        if s.game_crops != previous.game_crops {
            self.refresh_crops();
        }
        if s.split_windows != previous.split_windows {
            self.update_split_window();
        }
        self.sync_bottom_window();
        self.update_hdr();
        if s.auto_connect && !previous.auto_connect && self.connected.borrow().is_none() {
            self.capture.connect(None);
        }
        self.render_last_frame();
    }

    fn open_audio(&self) {
        let name = self.settings.borrow().audio_device.clone();
        let result = self.audio.borrow_mut().open((!name.is_empty()).then_some(name.as_str()));
        if let Err(e) = result {
            self.toast(&format!("Audio: {e}"));
        }
    }

    /// Centre of the main window, in desktop pixels.
    #[cfg(windows)]
    fn window_centre(&self) -> (i32, i32) {
        let window = self.ui.window();
        let (pos, size) = (window.position(), window.size());
        (pos.x + size.width as i32 / 2, pos.y + size.height as i32 / 2)
    }

    /// Text for the Video page about the display's HDR state.
    fn refresh_hdr_status(&self) {
        #[cfg(windows)]
        {
            let (x, y) = self.window_centre();
            let status = match hdr::display_at(x, y) {
                Some(d) if d.hdr_enabled => format!("HDR display, {:.0} nits peak. Shown when the menu is closed", d.max_nits),
                Some(_) => "Turn on \"Use HDR\" in Windows display settings first".to_string(),
                None => "No display information".to_string(),
            };
            self.ui.set_hdr_status(status.into());
        }
    }

    /// Show, hide, create or drop the HDR window to match the settings and menu.
    fn update_hdr(&self) {
        #[cfg(windows)]
        {
            let s = self.settings.borrow();
            if !s.hdr_output || self.kiosk {
                self.hdr.replace(None);
                self.hdr_visible.set(false);
                return;
            }
            let mut hdr = self.hdr.borrow_mut();
            let output = hdr.get_or_insert_with(|| {
                hdr::HdrOutput::new(|event| {
                    let _ = slint::invoke_from_event_loop(move || with_app(|app| app.handle_hdr_event(event)));
                })
            });
            output.settings(&s);
            let show = self.connected.borrow().is_some() && !self.menu_shown.get();
            if show && !self.hdr_visible.get() {
                let (x, y) = self.window_centre();
                output.show(x, y);
            } else if !show && self.hdr_visible.get() {
                output.hide();
            }
            self.hdr_visible.set(show);
        }
    }

    #[cfg(windows)]
    fn handle_hdr_event(&self, event: hdr::HdrEvent) {
        match event {
            hdr::HdrEvent::OpenMenu => {
                self.ui.set_menu_open(true);
                let _ = self.ui.show();
            }
            hdr::HdrEvent::Closed => {
                self.ui.set_hdr_output(false);
                self.ui.invoke_settings_changed();
            }
            hdr::HdrEvent::Error(e) => self.toast(&e),
        }
    }

    /// The split-screen window has its own copy of the display settings.
    fn sync_bottom_window(&self) {
        let Some(bottom) = &self.bottom else { return };
        let s = self.settings.borrow();
        bottom.set_integer_scaling(s.integer_scaling);
        bottom.set_smoothing(s.smoothing);
        bottom.set_filter_style(s.filter_style);
        bottom.set_filter_strength(s.filter_strength);
    }

    fn update_split_window(&self) {
        let Some(bottom) = &self.bottom else { return };
        let split = self.settings.borrow().split_windows;
        let result = if split { bottom.show() } else { bottom.hide() };
        if let Err(e) = result {
            self.toast(&format!("Window error: {e}"));
        }
    }

    fn toast(&self, message: &str) {
        self.ui.set_toast(message.into());
        let weak = self.ui.as_weak();
        let message = SharedString::from(message);
        slint::Timer::single_shot(Duration::from_millis(2500), move || {
            if let Some(ui) = weak.upgrade() {
                if ui.get_toast() == message {
                    ui.set_toast(SharedString::new());
                }
            }
        });
    }

    fn render_last_frame(&self) {
        let frame = self.last_frame.borrow();
        let Some(frame) = frame.as_ref() else { return };
        let s = self.settings.borrow();
        let crop_index = if frame.is_3ds { s.crop_3ds } else { s.crop_ds };
        let crop = view::crop_for(frame.is_3ds, s.game_crops, crop_index);
        let screens = view::render(frame, crop, s.top_rotation, s.bottom_rotation, s.mode_3d == 1);
        let top = screens.top.as_ref().map(view::plane_to_image).unwrap_or_default();
        let bottom = screens.bottom.as_ref().map(view::plane_to_image).unwrap_or_default();
        #[cfg(windows)]
        if self.hdr_visible.get() {
            if let Some(output) = self.hdr.borrow().as_ref() {
                output.frame(std::sync::Arc::new(hdr::HdrFrame { top: screens.top, bottom: screens.bottom }));
            }
        }
        self.ui.set_top_image(top);
        if let Some(window) = &self.bottom {
            window.set_bottom_image(bottom.clone());
        }
        self.ui.set_bottom_image(bottom);
    }

    fn clear_screens(&self) {
        self.last_frame.replace(None);
        self.ui.set_top_image(Image::default());
        self.ui.set_bottom_image(Image::default());
        if let Some(window) = &self.bottom {
            window.set_bottom_image(Image::default());
        }
    }

    fn handle_event(&self, event: Event) {
        let ui = &self.ui;
        match event {
            Event::Frame => {
                if let Some(frame) = self.capture.take_frame() {
                    self.last_frame.replace(Some(frame));
                    self.frame_count.set(self.frame_count.get() + 1);
                    self.render_last_frame();
                }
            }
            Event::Devices(devices) => {
                let items: Vec<DeviceItem> = devices
                    .iter()
                    .map(|d| DeviceItem {
                        name: d.display_name().into(),
                        detail: format!("{}{}", d.driver.label(), if d.superspeed { " · USB 3" } else { "" }).into(),
                    })
                    .collect();
                ui.set_devices(ModelRc::new(VecModel::from(items)));
                self.devices.replace(devices);
            }
            Event::Connecting(info) => {
                ui.set_connecting(true);
                ui.set_status_message(format!("Connecting to {}…", info.display_name()).into());
            }
            Event::Connected { info, can_3d } => {
                ui.set_connecting(false);
                ui.set_connected(true);
                ui.set_can_3d(can_3d);
                ui.set_is_3ds(info.board.is_3ds());
                ui.set_device_name(info.display_name().into());
                ui.set_device_driver(info.driver.label().into());
                ui.set_status_message(SharedString::new());
                ui.set_menu_open(false);
                ui.set_page(SharedString::new());
                self.connected.replace(Some(info));
                self.refresh_crops();
                self.update_hdr();
            }
            Event::Disconnected(reason) => {
                let was_connected = self.connected.replace(None).is_some();
                ui.set_connected(false);
                ui.set_connecting(false);
                ui.set_can_3d(false);
                ui.set_fps(0);
                if let Some(reason) = &reason {
                    ui.set_status_message(reason.as_str().into());
                } else if was_connected {
                    ui.set_status_message("Disconnected".into());
                }
                self.clear_screens();
                self.update_hdr();
            }
        }
    }
}

/// Runs a power command in kiosk mode. The kiosk installer grants the
/// service user passwordless sudo for exactly these commands.
fn power_command(action: &str) -> Result<(), String> {
    let args: &[&str] = match action {
        "poweroff" => &["-n", "/usr/bin/systemctl", "poweroff"],
        "reboot" => &["-n", "/usr/bin/systemctl", "reboot"],
        _ => return Err(format!("unknown power action {action}")),
    };
    let status = std::process::Command::new("sudo").args(args).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("sudo exited with {status}")) }
}

fn main() -> Result<(), slint::PlatformError> {
    // Kiosk mode: fullscreen on a single output, power controls instead of Quit.
    let kiosk = std::env::args().any(|a| a == "--kiosk");
    let ui = AppWindow::new()?;
    let bottom = if kiosk { None } else { Some(BottomWindow::new()?) };
    let mut settings = Settings::load();
    if kiosk {
        settings.split_windows = false;
    }
    ui.set_kiosk(kiosk);
    ui.set_hdr_supported(cfg!(windows) && !kiosk);

    let audio = AudioOutput::new();
    let mut audio_devices = vec!["Default".to_string()];
    audio_devices.extend(audio::output_device_names());
    ui.set_audio_devices(string_model(audio_devices.clone()));

    let weak = ui.as_weak();
    let capture = CaptureManager::new(audio.queue.clone(), settings.auto_connect, move |event| {
        let _ = weak.upgrade_in_event_loop(move |_| with_app(|app| app.handle_event(event)));
    });
    capture.set_3d(settings.enable_3d);
    audio.settings.set_volume(settings.volume as f32 / 100.0);
    audio.settings.set_muted(settings.muted);

    let app = Rc::new(App {
        ui: ui.clone_strong(),
        bottom: bottom.as_ref().map(|b| b.clone_strong()),
        kiosk,
        menu_shown: Cell::new(true),
        #[cfg(windows)]
        hdr: RefCell::new(None),
        #[cfg(windows)]
        hdr_visible: Cell::new(false),
        capture,
        audio: RefCell::new(audio),
        settings: RefCell::new(settings),
        devices: RefCell::new(Vec::new()),
        connected: RefCell::new(None),
        last_frame: RefCell::new(None),
        frame_count: Cell::new(0),
        audio_devices: RefCell::new(audio_devices),
    });
    APP.with(|a| *a.borrow_mut() = Some(app.clone()));

    app.push_settings_to_ui();
    app.sync_bottom_window();
    app.open_audio();
    ui.set_presets(ModelRc::new(VecModel::from(
        presets::PRESETS.iter().map(|p| DeviceItem { name: p.name.into(), detail: p.hint.into() }).collect::<Vec<_>>(),
    )));
    ui.set_profile_exists(ModelRc::new(VecModel::from((0..NUM_PROFILES).map(Settings::profile_exists).collect::<Vec<_>>())));

    ui.on_settings_changed(|| {
        with_app(|app| {
            let previous = app.settings.borrow().clone();
            app.pull_settings_from_ui();
            app.apply_settings(&previous);
        })
    });

    ui.on_connect_device(|index| {
        with_app(|app| {
            let serial = app.devices.borrow().get(index as usize).map(|d| d.serial.clone());
            app.capture.connect(serial);
        })
    });

    ui.on_disconnect(|| with_app(|app| app.capture.disconnect()));

    ui.on_menu_visibility_changed(|shown| {
        with_app(|app| {
            app.menu_shown.set(shown);
            if shown {
                app.refresh_hdr_status();
            }
            app.update_hdr();
        })
    });

    ui.on_toggle_fullscreen(|| {
        with_app(|app| {
            if app.kiosk {
                return;
            }
            let window = app.ui.window();
            let fullscreen = !window.is_fullscreen();
            window.set_fullscreen(fullscreen);
            app.ui.set_fullscreen(fullscreen);
        })
    });

    ui.on_save_profile(|slot| {
        with_app(|app| {
            let slot = slot as usize;
            match app.settings.borrow().save_profile(slot) {
                Ok(()) => app.toast(&format!("Profile {} saved", slot + 1)),
                Err(e) => app.toast(&format!("Could not save profile: {e}")),
            }
            app.ui.set_profile_exists(ModelRc::new(VecModel::from((0..NUM_PROFILES).map(Settings::profile_exists).collect::<Vec<_>>())));
        })
    });

    let load_settings = |new: Settings, message: String| {
        with_app(|app| {
            let previous = app.settings.replace(new);
            app.push_settings_to_ui();
            app.apply_settings(&previous);
            app.toast(&message);
        })
    };

    ui.on_load_profile(move |slot| {
        if let Some(profile) = Settings::load_profile(slot as usize) {
            load_settings(profile, format!("Profile {} loaded", slot + 1));
        }
    });

    ui.on_reset_settings(move || load_settings(Settings::default(), "Settings reset".into()));

    ui.on_apply_preset(move |index| {
        let Some(preset) = presets::PRESETS.get(index as usize) else { return };
        let mut settings = None;
        with_app(|app| settings = Some(app.settings.borrow().clone()));
        if let Some(mut s) = settings {
            preset.apply(&mut s);
            load_settings(s, format!("{} applied", preset.name));
        }
    });

    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    ui.on_power(|action| {
        with_app(|app| {
            if let Err(e) = power_command(action.as_str()) {
                app.toast(&format!("Could not {action}: {e}"));
            }
        })
    });

    ui.window().on_close_requested(|| {
        let _ = slint::quit_event_loop();
        CloseRequestResponse::HideWindow
    });

    if let Some(bottom) = &bottom {
        bottom.window().on_close_requested(|| {
            with_app(|app| {
                app.ui.set_split_windows(false);
                app.ui.invoke_settings_changed();
            });
            CloseRequestResponse::HideWindow
        });

        let f11 = SharedString::from(slint::platform::Key::F11);
        let escape = SharedString::from(slint::platform::Key::Escape);
        bottom.on_key(move |text| {
            with_app(|app| {
                if text == f11 {
                    if let Some(bottom) = &app.bottom {
                        let window = bottom.window();
                        window.set_fullscreen(!window.is_fullscreen());
                    }
                } else if text == escape {
                    app.ui.set_menu_open(true);
                    let _ = app.ui.show();
                }
            })
        });
    }

    // Clock and frame-rate counter.
    let clock_timer = slint::Timer::default();
    let update_clock = || {
        with_app(|app| {
            app.ui.set_clock(chrono::Local::now().format("%H:%M").to_string().into());
            app.ui.set_fps(app.frame_count.replace(0));
        })
    };
    update_clock();
    clock_timer.start(slint::TimerMode::Repeated, Duration::from_secs(1), update_clock);

    ui.show()?;
    app.update_split_window();
    if kiosk || app.settings.borrow().fullscreen {
        ui.window().set_fullscreen(true);
        ui.set_fullscreen(true);
    }
    slint::run_event_loop()?;

    // Remember fullscreen state for next launch (kiosk is always fullscreen).
    with_app(|app| {
        if !app.kiosk {
            app.settings.borrow_mut().fullscreen = app.ui.window().is_fullscreen();
        }
        let _ = app.settings.borrow().save();
    });
    APP.with(|a| a.borrow_mut().take());
    drop(app);
    Ok(())
}
