#![allow(unused)] // Suppresses ALL unused warnings
#![allow(dead_code, unused_imports, unused_variables)]
use crossterm::{
    cursor,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyCode, KeyModifiers,
    },
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{
        self, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use enigo::{Enigo, MouseButton, MouseControllable};
use notify_rust::Notification;
use rodio::{OutputStream, Sink, Source, source::SineWave};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Stdout, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tray_item::{IconSource, TrayItem};
use tui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color as TuiColor, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

// Windows API for global hotkeys
#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use std::ptr::null_mut;

#[cfg(windows)]
unsafe extern "system" {
    fn RegisterHotKey(hwnd: *mut c_void, id: i32, fsModifiers: u32, vk: u32) -> i32;
    fn UnregisterHotKey(hwnd: *mut c_void, id: i32) -> i32;
    fn PeekMessageW(
        lpMsg: *mut MSG,
        hWnd: *mut c_void,
        wMsgFilterMin: u32,
        wMsgFilterMax: u32,
        wRemoveMsg: u32,
    ) -> i32;
}

#[cfg(windows)]
#[repr(C)]
struct MSG {
    hwnd: *mut c_void,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: POINT,
}

#[cfg(windows)]
#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[cfg(windows)]
const WM_HOTKEY: u32 = 0x0312;
#[cfg(windows)]
const MOD_CONTROL: u32 = 0x0002;
#[cfg(windows)]
const MOD_SHIFT: u32 = 0x0004;
#[cfg(windows)]
const MOD_ALT: u32 = 0x0001;
#[cfg(windows)]
const PM_REMOVE: u32 = 0x0001;

// FIXED: Event system for responsive input handling
#[derive(Debug)]
enum AppEvent {
    Input(crossterm::event::KeyEvent),
    Tick,
    Quit,
}

// Professional theme only
#[derive(Clone, Debug)]
pub struct Theme {
    pub primary: TuiColor,
    pub secondary: TuiColor,
    pub accent: TuiColor,
    pub text: TuiColor,
    pub success: TuiColor,
    pub warning: TuiColor,
    pub error: TuiColor,
}

impl Theme {
    pub fn professional() -> Self {
        Self {
            primary: TuiColor::Rgb(70, 130, 180),
            secondary: TuiColor::Rgb(105, 105, 105),
            accent: TuiColor::Rgb(255, 165, 0),
            text: TuiColor::Rgb(220, 220, 220),
            success: TuiColor::Rgb(34, 139, 34),
            warning: TuiColor::Rgb(255, 140, 0),
            error: TuiColor::Rgb(220, 20, 60),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct KeyCombo {
    mods: u8,
    key: String,
}

impl std::fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if self.mods & 2 != 0 {
            parts.push("Ctrl");
        }
        if self.mods & 1 != 0 {
            parts.push("Shift");
        }
        if self.mods & 4 != 0 {
            parts.push("Alt");
        }
        parts.push(&self.key);
        write!(f, "{}", parts.join("+"))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Statistics {
    total_clicks: u64,
    session_clicks: u64,
    total_sessions: u64,
    last_session_start: u64,
    session_duration: u64,
}

impl Default for Statistics {
    fn default() -> Self {
        Self {
            total_clicks: 0,
            session_clicks: 0,
            total_sessions: 0,
            last_session_start: 0,
            session_duration: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Config {
    cps_presets: Vec<u32>,
    selected_preset: usize,
    custom_cps_value: Option<u32>,
    using_custom_cps: bool,
    selected_button: usize,
    toggle_keybind: Option<KeyCombo>,
    statistics: Statistics,
    sound_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cps_presets: vec![20, 30, 40, 50],
            selected_preset: 0,
            custom_cps_value: None,
            using_custom_cps: false,
            selected_button: 0,
            toggle_keybind: Some(KeyCombo {
                mods: 6, // Ctrl+Shift
                key: "B".to_string(),
            }),
            statistics: Statistics::default(),
            sound_enabled: true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Normal,
    EditingCps,
    SettingKeybind,
    AwaitingKeybind,
    ShowingHelp,
}

#[allow(dead_code)]
struct TrayManager {
    tray: TrayItem,
    flash_active: Arc<AtomicBool>,
    flash_handle: Option<thread::JoinHandle<()>>,
}

impl TrayManager {
    fn new(show_tui: Arc<AtomicBool>, auto_clicker_running: Arc<AtomicBool>) -> Option<Self> {
        let show_tui_clone = Arc::clone(&show_tui);
        let running_clone = Arc::clone(&auto_clicker_running);

        let mut tray = TrayItem::new("BClicker Pro", IconSource::Resource("")).ok()?;

        tray.add_menu_item("Show Interface", move || {
            show_tui_clone.store(true, Ordering::SeqCst);
        })
        .ok()?;

        tray.add_menu_item("Toggle Clicker", move || {
            let current = running_clone.load(Ordering::SeqCst);
            running_clone.store(!current, Ordering::SeqCst);
        })
        .ok()?;

        tray.add_menu_item("Exit", || {
            std::process::exit(0);
        })
        .ok()?;

        Some(Self {
            tray,
            flash_active: Arc::new(AtomicBool::new(false)),
            flash_handle: None,
        })
    }

    fn start_flashing(&mut self) {
        self.flash_active.store(true, Ordering::SeqCst);
        let flash_active = Arc::clone(&self.flash_active);

        self.flash_handle = Some(thread::spawn(move || {
            let mut toggle = false;
            while flash_active.load(Ordering::SeqCst) {
                toggle = !toggle;
                thread::sleep(Duration::from_millis(500));
            }
        }));
    }

    fn stop_flashing(&mut self) {
        self.flash_active.store(false, Ordering::SeqCst);
        if let Some(handle) = self.flash_handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Clone)]
struct AudioManager {
    enabled: bool,
}

impl AudioManager {
    fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    fn play_start_sound(&self) {
        if !self.enabled {
            return;
        }

        thread::spawn(|| {
            if let Ok((_stream, stream_handle)) = OutputStream::try_default() {
                if let Ok(sink) = Sink::try_new(&stream_handle) {
                    let source = SineWave::new(880.0)
                        .take_duration(Duration::from_millis(200))
                        .amplify(0.1);
                    sink.append(source);
                    sink.sleep_until_end();
                }
            }
        });
    }

    fn play_stop_sound(&self) {
        if !self.enabled {
            return;
        }

        thread::spawn(|| {
            if let Ok((_stream, stream_handle)) = OutputStream::try_default() {
                if let Ok(sink) = Sink::try_new(&stream_handle) {
                    let source = SineWave::new(440.0)
                        .take_duration(Duration::from_millis(150))
                        .amplify(0.1);
                    sink.append(source);
                    sink.sleep_until_end();
                }
            }
        });
    }

    fn toggle_sound(&mut self) {
        self.enabled = !self.enabled;
    }
}

struct App {
    config: Config,
    auto_clicker_running: Arc<AtomicBool>,
    custom_cps_input: String,
    input_mode: InputMode,
    keybind_wait_start: Option<Instant>,
    session_start: Instant,
    #[allow(dead_code)]
    tray_manager: Option<TrayManager>,
    show_tui: Arc<AtomicBool>,
    current_cps: Arc<Mutex<u32>>,
    current_button: Arc<Mutex<usize>>,
    stats_tracker: Arc<Mutex<Statistics>>,
    theme: Theme,
    audio_manager: AudioManager,
    help_scroll: usize,
    should_quit: bool,
    needs_redraw: bool, // FIXED: Only redraw when needed
}

impl App {
    fn new() -> Self {
        let mut config = load_config();
        config.statistics.total_sessions += 1;
        config.statistics.session_clicks = 0;
        config.statistics.last_session_start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let current_cps = if config.using_custom_cps {
            config.custom_cps_value.unwrap_or(20)
        } else {
            config
                .cps_presets
                .get(config.selected_preset)
                .copied()
                .unwrap_or(20)
        };

        let theme = Theme::professional();
        let audio_manager = AudioManager::new(config.sound_enabled);

        Self {
            config: config.clone(),
            auto_clicker_running: Arc::new(AtomicBool::new(false)),
            custom_cps_input: String::new(),
            input_mode: InputMode::Normal,
            keybind_wait_start: None,
            session_start: Instant::now(),
            tray_manager: None,
            show_tui: Arc::new(AtomicBool::new(true)),
            current_cps: Arc::new(Mutex::new(current_cps)),
            current_button: Arc::new(Mutex::new(config.selected_button)),
            stats_tracker: Arc::new(Mutex::new(config.statistics)),
            theme,
            audio_manager,
            help_scroll: 0,
            should_quit: false,
            needs_redraw: true,
        }
    }

    fn save_config(&mut self) {
        if let Ok(stats) = self.stats_tracker.lock() {
            self.config.statistics = stats.clone();
        }
        save_config(&self.config);
    }

    fn get_current_cps(&self) -> u32 {
        *self.current_cps.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn update_cps(&mut self) {
        let new_cps = if self.config.using_custom_cps {
            self.config.custom_cps_value.unwrap_or(20)
        } else {
            self.config
                .cps_presets
                .get(self.config.selected_preset)
                .copied()
                .unwrap_or(20)
        };
        *self.current_cps.lock().unwrap() = new_cps;
        self.needs_redraw = true;
    }

    fn get_current_button_text(&self) -> &'static str {
        match self.config.selected_button {
            0 => "Left Click",
            1 => "Right Click",
            _ => "Left Click",
        }
    }

    fn cycle_button(&mut self) {
        self.config.selected_button = (self.config.selected_button + 1) % 2;
        *self.current_button.lock().unwrap() = self.config.selected_button;
        self.needs_redraw = true;
    }

    fn move_selection_up(&mut self) {
        if self.config.using_custom_cps {
            self.config.using_custom_cps = false;
            self.config.selected_preset = self.config.cps_presets.len() - 1;
        } else {
            if self.config.selected_preset > 0 {
                self.config.selected_preset -= 1;
            } else {
                if self.config.custom_cps_value.is_some() {
                    self.config.using_custom_cps = true;
                } else {
                    self.config.selected_preset = self.config.cps_presets.len() - 1;
                }
            }
        }
        self.update_cps();
    }

    fn move_selection_down(&mut self) {
        if self.config.using_custom_cps {
            self.config.using_custom_cps = false;
            self.config.selected_preset = 0;
        } else {
            if self.config.selected_preset + 1 < self.config.cps_presets.len() {
                self.config.selected_preset += 1;
            } else {
                if self.config.custom_cps_value.is_some() {
                    self.config.using_custom_cps = true;
                } else {
                    self.config.selected_preset = 0;
                }
            }
        }
        self.update_cps();
    }

    fn show_notification(&self, title: &str, message: &str) {
        let _ = Notification::new()
            .summary(title)
            .body(message)
            .timeout(3000)
            .show();
    }

    // FIXED: Fast input handling without lag
    fn handle_input(&mut self, key_event: crossterm::event::KeyEvent) {
        match self.input_mode {
            InputMode::ShowingHelp => match key_event.code {
                KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q') => {
                    self.input_mode = InputMode::Normal;
                    self.needs_redraw = true;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.help_scroll < 20 {
                        self.help_scroll += 1;
                        self.needs_redraw = true;
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.help_scroll > 0 {
                        self.help_scroll -= 1;
                        self.needs_redraw = true;
                    }
                }
                _ => {}
            },
            InputMode::Normal => {
                match key_event.code {
                    KeyCode::Char('q') => {
                        self.should_quit = true;
                    }
                    KeyCode::Char('?') => {
                        self.input_mode = InputMode::ShowingHelp;
                        self.help_scroll = 0;
                        self.needs_redraw = true;
                    }
                    KeyCode::Char('h') => {
                        // FIXED: Toggle hide/show without freeze
                        let current = self.show_tui.load(Ordering::SeqCst);
                        self.show_tui.store(!current, Ordering::SeqCst);
                        self.show_notification(
                            "BClicker",
                            if current {
                                "Hidden to system tray"
                            } else {
                                "Interface shown"
                            },
                        );
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.move_selection_down();
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.move_selection_up();
                    }
                    KeyCode::Char('e') => {
                        self.input_mode = InputMode::EditingCps;
                        self.custom_cps_input.clear();
                        self.needs_redraw = true;
                    }
                    KeyCode::Char('s') => {
                        self.input_mode = InputMode::AwaitingKeybind;
                        self.keybind_wait_start = Some(Instant::now());
                        self.needs_redraw = true;
                    }
                    KeyCode::Tab => {
                        self.cycle_button();
                    }
                    KeyCode::Char('m') => {
                        self.audio_manager.toggle_sound();
                        self.config.sound_enabled = self.audio_manager.enabled;
                        let status = if self.audio_manager.enabled {
                            "enabled"
                        } else {
                            "disabled"
                        };
                        self.show_notification("Audio", &format!("Sound effects {}", status));
                        self.needs_redraw = true;
                    }
                    KeyCode::Char('r') => {
                        if let Ok(mut stats) = self.stats_tracker.lock() {
                            *stats = Statistics::default();
                            self.session_start = Instant::now();
                        }
                        self.show_notification("Statistics", "Statistics reset");
                        self.needs_redraw = true;
                    }
                    _ => {}
                }
            }
            InputMode::EditingCps => match key_event.code {
                KeyCode::Enter => {
                    if let Ok(val) = self.custom_cps_input.trim().parse::<u32>() {
                        if val > 0 && val <= 1000 {
                            self.config.custom_cps_value = Some(val);
                            self.config.using_custom_cps = true;
                            self.update_cps();
                            self.show_notification(
                                "CPS Updated",
                                &format!("Custom CPS set to: {}", val),
                            );
                        }
                    }
                    self.input_mode = InputMode::Normal;
                    self.needs_redraw = true;
                }
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    if self.custom_cps_input.len() < 3 {
                        self.custom_cps_input.push(c);
                        self.needs_redraw = true;
                    }
                }
                KeyCode::Backspace => {
                    self.custom_cps_input.pop();
                    self.needs_redraw = true;
                }
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.custom_cps_input.clear();
                    self.needs_redraw = true;
                }
                _ => {}
            },
            InputMode::AwaitingKeybind => {
                if let Some(wait_start) = self.keybind_wait_start {
                    if Instant::now().duration_since(wait_start) > Duration::from_millis(800) {
                        self.input_mode = InputMode::SettingKeybind;
                        self.keybind_wait_start = None;
                        self.needs_redraw = true;
                    }
                }
            }
            InputMode::SettingKeybind => match key_event.code {
                KeyCode::Char(c) => {
                    let mut mods = 0u8;
                    if key_event.modifiers.contains(KeyModifiers::CONTROL) {
                        mods |= 2;
                    }
                    if key_event.modifiers.contains(KeyModifiers::SHIFT) {
                        mods |= 1;
                    }
                    if key_event.modifiers.contains(KeyModifiers::ALT) {
                        mods |= 4;
                    }

                    self.config.toggle_keybind = Some(KeyCombo {
                        mods,
                        key: c.to_ascii_uppercase().to_string(),
                    });
                    self.input_mode = InputMode::Normal;
                    self.show_notification(
                        "Hotkey Updated",
                        &format!(
                            "New hotkey: {}",
                            self.config.toggle_keybind.as_ref().unwrap()
                        ),
                    );
                    self.needs_redraw = true;
                }
                KeyCode::F(n) => {
                    let mut mods = 0u8;
                    if key_event.modifiers.contains(KeyModifiers::CONTROL) {
                        mods |= 2;
                    }
                    if key_event.modifiers.contains(KeyModifiers::SHIFT) {
                        mods |= 1;
                    }
                    if key_event.modifiers.contains(KeyModifiers::ALT) {
                        mods |= 4;
                    }

                    self.config.toggle_keybind = Some(KeyCombo {
                        mods,
                        key: format!("F{}", n),
                    });
                    self.input_mode = InputMode::Normal;
                    self.show_notification(
                        "Hotkey Updated",
                        &format!(
                            "New hotkey: {}",
                            self.config.toggle_keybind.as_ref().unwrap()
                        ),
                    );
                    self.needs_redraw = true;
                }
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.needs_redraw = true;
                }
                _ => {}
            },
        }

        self.save_config();
    }

    fn update(&mut self) {
        // Update any time-based state changes
        if self.input_mode == InputMode::AwaitingKeybind {
            if let Some(wait_start) = self.keybind_wait_start {
                if Instant::now().duration_since(wait_start) > Duration::from_millis(800) {
                    self.input_mode = InputMode::SettingKeybind;
                    self.keybind_wait_start = None;
                    self.needs_redraw = true;
                }
            }
        }
    }
}

// FIXED: Optimized hotkey display function with proper lifetimes
fn create_hotkey_spans<'a>(keybind: &'a KeyCombo, theme: &'a Theme) -> Vec<Span<'a>> {
    let mut spans = Vec::new();

    if keybind.mods & 2 != 0 {
        spans.push(Span::styled(
            "Ctrl",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw("+"));
    }
    if keybind.mods & 4 != 0 {
        spans.push(Span::styled(
            "Alt",
            Style::default()
                .fg(theme.success)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw("+"));
    }
    if keybind.mods & 1 != 0 {
        spans.push(Span::styled(
            "Shift",
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw("+"));
    }

    spans.push(Span::styled(
        &keybind.key,
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    ));
    spans
}

fn get_config_path() -> PathBuf {
    let mut path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    path.push("bclicker_config.toml");
    path
}

fn load_config() -> Config {
    let path = get_config_path();
    match fs::read_to_string(&path) {
        Ok(contents) => toml::from_str(&contents).unwrap_or_else(|_| {
            println!("Warning: Invalid config file, using defaults");
            Config::default()
        }),
        Err(_) => {
            println!("Config file not found, creating with defaults");
            Config::default()
        }
    }
}

fn save_config(config: &Config) {
    let path = get_config_path();
    match toml::to_string_pretty(config) {
        Ok(contents) => {
            if let Err(e) = fs::write(&path, contents) {
                eprintln!("Warning: Could not save config: {}", e);
            }
        }
        Err(e) => {
            eprintln!("Warning: Could not serialize config: {}", e);
        }
    }
}

#[cfg(windows)]
fn setup_global_hotkey(
    config: &Config,
    auto_clicker_running: Arc<AtomicBool>,
) -> Option<thread::JoinHandle<()>> {
    if let Some(keybind) = config.toggle_keybind.clone() {
        let running_flag = auto_clicker_running.clone();
        let mods = keybind.mods;
        let key = keybind.key.clone();

        Some(thread::spawn(move || {
            let mut modifiers = 0u32;
            if mods & 2 != 0 {
                modifiers |= MOD_CONTROL;
            }
            if mods & 1 != 0 {
                modifiers |= MOD_SHIFT;
            }
            if mods & 4 != 0 {
                modifiers |= MOD_ALT;
            }

            let vk_code = match key.as_str() {
                "A" => 0x41,
                "B" => 0x42,
                "C" => 0x43,
                "D" => 0x44,
                "E" => 0x45,
                "F" => 0x46,
                "G" => 0x47,
                "H" => 0x48,
                "I" => 0x49,
                "J" => 0x4A,
                "K" => 0x4B,
                "L" => 0x4C,
                "M" => 0x4D,
                "N" => 0x4E,
                "O" => 0x4F,
                "P" => 0x50,
                "Q" => 0x51,
                "R" => 0x52,
                "S" => 0x53,
                "T" => 0x54,
                "U" => 0x55,
                "V" => 0x56,
                "W" => 0x57,
                "X" => 0x58,
                "Y" => 0x59,
                "Z" => 0x5A,
                "F1" => 0x70,
                "F2" => 0x71,
                "F3" => 0x72,
                "F4" => 0x73,
                "F5" => 0x74,
                "F6" => 0x75,
                "F7" => 0x76,
                "F8" => 0x77,
                "F9" => 0x78,
                "F10" => 0x79,
                "F11" => 0x7A,
                "F12" => 0x7B,
                _ => 0x42,
            };

            let hotkey_id = 1;
            let result = unsafe { RegisterHotKey(null_mut(), hotkey_id, modifiers, vk_code) };

            if result != 0 {
                println!(
                    "[INFO] Global hotkey registered: {}",
                    format!("{}", KeyCombo { mods, key })
                );

                loop {
                    let mut msg: MSG = unsafe { std::mem::zeroed() };
                    let result = unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) };

                    if result != 0 && msg.message == WM_HOTKEY && msg.wparam == hotkey_id as usize {
                        let current = running_flag.load(Ordering::SeqCst);
                        running_flag.store(!current, Ordering::SeqCst);
                    }

                    thread::sleep(Duration::from_millis(10));
                }
            } else {
                eprintln!("[ERROR] Failed to register global hotkey");
            }
        }))
    } else {
        None
    }
}

#[cfg(not(windows))]
fn setup_global_hotkey(
    _config: &Config,
    _auto_clicker_running: Arc<AtomicBool>,
) -> Option<thread::JoinHandle<()>> {
    println!("[WARNING] Global hotkeys only supported on Windows");
    None
}

fn start_clicker_thread(
    auto_clicker_running: Arc<AtomicBool>,
    current_cps: Arc<Mutex<u32>>,
    current_button: Arc<Mutex<usize>>,
    stats_tracker: Arc<Mutex<Statistics>>,
    audio_manager: Arc<Mutex<AudioManager>>,
    tray_manager: Arc<Mutex<Option<TrayManager>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut enigo = Enigo::new();
        let mut last_click_time = Instant::now();
        let mut was_running = false;

        loop {
            let is_running = auto_clicker_running.load(Ordering::SeqCst);

            if is_running != was_running {
                if let Ok(audio) = audio_manager.lock() {
                    if is_running {
                        audio.play_start_sound();
                    } else {
                        audio.play_stop_sound();
                    }
                }

                if let Ok(mut tray) = tray_manager.lock() {
                    if let Some(tray) = tray.as_mut() {
                        if is_running {
                            tray.start_flashing();
                        } else {
                            tray.stop_flashing();
                        }
                    }
                }

                was_running = is_running;
            }

            if is_running {
                let cps = *current_cps.lock().unwrap_or_else(|e| e.into_inner());
                let button_idx = *current_button.lock().unwrap_or_else(|e| e.into_inner());

                let mouse_btn = match button_idx {
                    0 => MouseButton::Left,
                    1 => MouseButton::Right,
                    _ => MouseButton::Left,
                };

                let target_delay = Duration::from_micros(1_000_000 / cps as u64);
                let elapsed = last_click_time.elapsed();

                if elapsed >= target_delay {
                    enigo.mouse_click(mouse_btn);

                    if let Ok(mut stats) = stats_tracker.lock() {
                        stats.total_clicks += 1;
                        stats.session_clicks += 1;
                    }

                    last_click_time = Instant::now();
                } else {
                    let remaining = target_delay - elapsed;
                    if remaining > Duration::from_millis(1) {
                        thread::sleep(remaining);
                    }
                }
            } else {
                thread::sleep(Duration::from_millis(50));
            }
        }
    })
}

// FIXED: Fast event handling system without blocking
fn setup_event_system() -> (mpsc::Sender<AppEvent>, mpsc::Receiver<AppEvent>) {
    let (tx, rx) = mpsc::channel();
    let tx_clone = tx.clone();

    // Input handling thread - no more lag!
    thread::spawn(move || {
        loop {
            if let Ok(CEvent::Key(key)) = event::read() {
                if tx_clone.send(AppEvent::Input(key)).is_err() {
                    break;
                }
            }
        }
    });

    // Tick thread for smooth updates
    let tx_tick = tx.clone();
    thread::spawn(move || {
        let tick_rate = Duration::from_millis(16); // ~60 FPS
        loop {
            thread::sleep(tick_rate);
            if tx_tick.send(AppEvent::Tick).is_err() {
                break;
            }
        }
    });

    (tx, rx)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    loading_animation()?;

    let mut app = App::new();
    println!(
        "[INIT] BClicker Professional initialized (Session #{})",
        app.config.statistics.total_sessions
    );

    let tray_manager = TrayManager::new(
        Arc::clone(&app.show_tui),
        Arc::clone(&app.auto_clicker_running),
    );
    let tray_manager_arc = Arc::new(Mutex::new(tray_manager));

    app.show_notification(
        "BClicker Professional",
        "Started successfully! Use global hotkey to toggle.",
    );

    let _hotkey_handle = setup_global_hotkey(&app.config, Arc::clone(&app.auto_clicker_running));

    let audio_manager = Arc::new(Mutex::new(app.audio_manager.clone()));

    let _clicker_handle = start_clicker_thread(
        Arc::clone(&app.auto_clicker_running),
        Arc::clone(&app.current_cps),
        Arc::clone(&app.current_button),
        Arc::clone(&app.stats_tracker),
        Arc::clone(&audio_manager),
        Arc::clone(&tray_manager_arc),
    );

    println!("[SUCCESS] BClicker Professional started successfully");

    // FIXED: Fast event system setup
    let (_tx, rx) = setup_event_system();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // FIXED: Main loop with no lag and proper hide/show toggle
    loop {
        if app.should_quit {
            break;
        }

        // Handle events without blocking
        while let Ok(event) = rx.try_recv() {
            match event {
                AppEvent::Input(key_event) => {
                    // Only process input when UI is shown
                    if app.show_tui.load(Ordering::SeqCst) {
                        app.handle_input(key_event);
                    }
                }
                AppEvent::Tick => {
                    app.update();
                }
                AppEvent::Quit => {
                    app.should_quit = true;
                }
            }
        }

        // Only draw when UI is shown AND needs redraw - no more lag!
        if app.show_tui.load(Ordering::SeqCst) && app.needs_redraw {
            terminal.draw(|f| {
                if app.input_mode == InputMode::ShowingHelp {
                    draw_help_screen(f, &app);
                } else {
                    draw_ui(f, &app);
                }
            })?;
            app.needs_redraw = false;
        }

        // Small sleep when hidden to reduce CPU usage
        if !app.show_tui.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(100));
        } else {
            thread::sleep(Duration::from_millis(16)); // ~60 FPS
        }
    }

    app.save_config();
    cleanup_terminal(&mut terminal)?;
    println!("[EXIT] BClicker Professional closed. Configuration saved.");
    Ok(())
}

// FIXED: Simplified and perfectly centered loading animation
fn loading_animation() -> Result<(), Box<dyn std::error::Error>> {
    let mut stdout = io::stdout();
    execute!(stdout, terminal::Clear(ClearType::All), cursor::Hide)?;

    let term_size = terminal::size()?;
    let term_width = term_size.0 as usize;
    let term_height = term_size.1 as usize;

    let title = "BClicker Professional v2.0";
    let warning = "⚠️  WARNING: Using auto-clicker in games may result in account bans  ⚠️";

    let spinner_chars = vec!["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    let center_y = term_height / 2;

    for i in 0..40 {
        execute!(
            stdout,
            cursor::MoveTo(0, 0),
            terminal::Clear(ClearType::All)
        )?;

        // Centered title
        let title_x = if term_width > title.len() {
            (term_width - title.len()) / 2
        } else {
            0
        };
        execute!(
            stdout,
            cursor::MoveTo(title_x as u16, (center_y.saturating_sub(3)) as u16),
            SetForegroundColor(Color::Cyan),
            Print(title),
            ResetColor
        )?;

        // Centered spinner
        let spinner_idx = i % spinner_chars.len();
        let spinner = spinner_chars[spinner_idx];
        let loader_text = format!("Loading {}", spinner);
        let loader_x = if term_width > loader_text.len() {
            (term_width - loader_text.len()) / 2
        } else {
            0
        };
        execute!(
            stdout,
            cursor::MoveTo(loader_x as u16, center_y as u16),
            SetForegroundColor(Color::Green),
            Print(loader_text),
            ResetColor
        )?;

        // Centered warning
        let warning_x = if term_width > warning.len() {
            (term_width - warning.len()) / 2
        } else {
            0
        };
        execute!(
            stdout,
            cursor::MoveTo(warning_x as u16, (center_y + 2) as u16),
            SetForegroundColor(Color::Red),
            Print(warning),
            ResetColor
        )?;

        stdout.flush()?;
        thread::sleep(Duration::from_millis(100));
    }

    let complete_msg = "Loading Complete! Press 'h' to hide, '?' for help";
    let complete_x = if term_width > complete_msg.len() {
        (term_width - complete_msg.len()) / 2
    } else {
        0
    };
    execute!(
        stdout,
        cursor::MoveTo(complete_x as u16, (center_y + 4) as u16),
        SetForegroundColor(Color::Green),
        Print(complete_msg),
        ResetColor
    )?;

    stdout.flush()?;
    thread::sleep(Duration::from_millis(1500));

    execute!(stdout, cursor::Show)?;
    Ok(())
}

fn cleanup_terminal(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        cursor::Show
    )?;
    terminal.show_cursor()?;

    #[cfg(windows)]
    unsafe {
        UnregisterHotKey(null_mut(), 1);
    }

    Ok(())
}

// FIXED: Beautiful and responsive help screen
fn draw_help_screen<B: Backend>(f: &mut tui::Frame<B>, app: &App) {
    let area = f.size();

    let help_text = vec![
        "",
        "╔══════════════════════════════════════════════════════════════╗",
        "║                  BClicker Professional v2.0                 ║",
        "║                     Help & Documentation                     ║",
        "╚══════════════════════════════════════════════════════════════╝",
        "",
        "🎯 MAIN CONTROLS:",
        "   ↑/↓ or j/k      Navigate CPS presets",
        "   Tab              Switch Left/Right click modes",
        "   Enter            Confirm selection",
        "   Esc              Cancel operation",
        "",
        "⚡ PRIMARY FUNCTIONS:",
        "   E                Edit custom CPS (1-1000)",
        "   S                Set global hotkey",
        "   H                Hide/Show interface (toggle)",
        "   Q                Quit and save",
        "   ?                Toggle this help screen",
        "",
        "🔧 ADVANCED FEATURES:",
        "   M                Toggle sound effects",
        "   R                Reset session statistics",
        "",
        "🎮 GLOBAL HOTKEY:",
        "   Your hotkey works system-wide to start/stop clicking",
        "   Default: Ctrl+Shift+B",
        "   Works even when interface is hidden",
        "",
        "📊 SYSTEM TRAY:",
        "   • Right-click tray icon for menu",
        "   • Icon flashes when clicker is active",
        "   • Notifications keep you informed",
        "",
        "🔊 AUDIO FEEDBACK:",
        "   • Start/stop sound effects",
        "   • Toggle with 'M' key",
        "   • Professional audio cues",
        "",
        "💾 CONFIGURATION:",
        "   • Auto-saves to bclicker_config.toml",
        "   • Statistics tracked across sessions",
        "   • All preferences persist",
        "",
        "📈 STATISTICS TRACKING:",
        "   • Real-time session tracking",
        "   • Total clicks across sessions",
        "   • Average CPS calculation",
        "",
        "⚠️  IMPORTANT NOTICE:",
        "   Using auto-clickers in competitive games",
        "   may violate terms of service and result",
        "   in account bans. Use responsibly!",
        "",
        "────────────────────────────────────────────────────────────────",
        "Use ↑/↓ to scroll • Press ? or Esc to close help",
    ];

    let visible_lines = area.height.saturating_sub(2) as usize;
    let start_line = app.help_scroll;
    let end_line = (start_line + visible_lines).min(help_text.len());

    let help_content: Vec<Spans> = help_text[start_line..end_line]
        .iter()
        .map(|line| {
            if line.contains("BClicker Professional") {
                Spans::from(Span::styled(
                    *line,
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ))
            } else if line.starts_with("🎯") || line.starts_with("⚡") || line.starts_with("🔧")
            {
                Spans::from(Span::styled(
                    *line,
                    Style::default()
                        .fg(app.theme.primary)
                        .add_modifier(Modifier::BOLD),
                ))
            } else if line.starts_with("   ") && line.len() > 3 {
                let parts: Vec<&str> = line.splitn(2, ' ').collect();
                if parts.len() >= 2 {
                    Spans::from(vec![
                        Span::styled(
                            parts[0],
                            Style::default()
                                .fg(app.theme.success)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(&line[parts[0].len()..], Style::default().fg(app.theme.text)),
                    ])
                } else {
                    Spans::from(Span::styled(*line, Style::default().fg(app.theme.text)))
                }
            } else if line.starts_with("⚠️") {
                Spans::from(Span::styled(
                    *line,
                    Style::default()
                        .fg(app.theme.error)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Spans::from(Span::styled(*line, Style::default().fg(app.theme.text)))
            }
        })
        .collect();

    let help_widget = Paragraph::new(help_content)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " BClicker Professional - Help System ",
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(app.theme.primary)),
        )
        .alignment(Alignment::Left);

    f.render_widget(help_widget, area);
}

// FIXED: Optimized and responsive UI with better layout
fn draw_ui<B: Backend>(f: &mut tui::Frame<B>, app: &App) {
    let size = f.size();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Length(3), // Status
                Constraint::Min(5),    // CPS Selection
                Constraint::Length(3), // Custom Input
                Constraint::Length(4), // Statistics
                Constraint::Length(4), // Instructions
            ]
            .as_ref(),
        )
        .split(size);

    // Status bar with clean hotkey display
    let running_status = if app.auto_clicker_running.load(Ordering::SeqCst) {
        Span::styled(
            "🟢 ACTIVE",
            Style::default()
                .fg(app.theme.success)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            "🔴 IDLE",
            Style::default()
                .fg(app.theme.secondary)
                .add_modifier(Modifier::BOLD),
        )
    };

    let mut status_spans = vec![running_status, Span::raw(" │ Hotkey: ")];

    if let Some(keybind) = &app.config.toggle_keybind {
        status_spans.extend(create_hotkey_spans(keybind, &app.theme));
    } else {
        status_spans.push(Span::styled(
            "Not Set",
            Style::default().fg(app.theme.error),
        ));
    }

    let current_cps = app.get_current_cps();
    let button_text = app.get_current_button_text();

    status_spans.extend(vec![
        Span::raw(" │ Button: "),
        Span::styled(
            button_text,
            Style::default()
                .fg(app.theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" │ {} CPS", current_cps)),
    ]);

    let status = Paragraph::new(Spans::from(status_spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " BClicker Professional v2.0 ",
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(app.theme.primary)),
    );

    f.render_widget(status, chunks[0]);

    // CPS Selection with better visual indicators
    let mut cps_items: Vec<ListItem> = app
        .config
        .cps_presets
        .iter()
        .enumerate()
        .map(|(i, &cps)| {
            let selected = i == app.config.selected_preset && !app.config.using_custom_cps;
            let style = if selected {
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.theme.text)
            };
            let prefix = if selected { "▶ " } else { "  " };
            ListItem::new(format!("{}{} CPS", prefix, cps)).style(style)
        })
        .collect();

    if let Some(custom_cps) = app.config.custom_cps_value {
        let selected = app.config.using_custom_cps;
        let style = if selected {
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(app.theme.secondary)
        };
        let prefix = if selected { "▶ " } else { "  " };
        cps_items
            .push(ListItem::new(format!("{}{} CPS (Custom)", prefix, custom_cps)).style(style));
    }

    let cps_list = List::new(cps_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " ⚡ Click Speed Configuration ",
                Style::default().fg(app.theme.primary),
            ))
            .border_style(Style::default().fg(app.theme.secondary)),
    );

    f.render_widget(cps_list, chunks[1]);

    // Enhanced input field
    let input_style = match app.input_mode {
        InputMode::EditingCps => Style::default()
            .fg(app.theme.accent)
            .add_modifier(Modifier::BOLD),
        _ => Style::default().fg(app.theme.secondary),
    };

    let input_title = match app.input_mode {
        InputMode::EditingCps => " 📝 Custom CPS Input [Type 1-1000, Enter to save] ",
        _ => " 📝 Custom CPS Input [Press E to edit] ",
    };

    let input_text = if app.input_mode == InputMode::EditingCps {
        format!("{}_", &app.custom_cps_input)
    } else {
        "".to_string()
    };

    let input_block = Paragraph::new(input_text).style(input_style).block(
        Block::default()
            .borders(Borders::ALL)
            .title(input_title)
            .border_style(Style::default().fg(app.theme.secondary)),
    );

    f.render_widget(input_block, chunks[2]);

    // Compact statistics
    let stats = app.stats_tracker.lock().unwrap_or_else(|e| e.into_inner());
    let session_duration = app.session_start.elapsed().as_secs();
    let session_cps = if session_duration > 0 {
        stats.session_clicks / session_duration
    } else {
        0
    };

    let stats_content = vec![
        Spans::from(format!(
            "📊 Session: {} clicks in {}m {}s (avg {} CPS)",
            stats.session_clicks,
            session_duration / 60,
            session_duration % 60,
            session_cps
        )),
        Spans::from(format!(
            "🎯 Total: {} clicks │ Sessions: {} │ Audio: {}",
            stats.total_clicks,
            stats.total_sessions,
            if app.config.sound_enabled {
                "🔊 On"
            } else {
                "🔇 Off"
            }
        )),
    ];

    let stats_widget = Paragraph::new(stats_content)
        .style(Style::default().fg(app.theme.text))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " 📈 Statistics ",
                    Style::default().fg(app.theme.primary),
                ))
                .border_style(Style::default().fg(app.theme.secondary)),
        );

    f.render_widget(stats_widget, chunks[3]);

    // Dynamic instructions based on mode
    let instruction_color = match app.input_mode {
        InputMode::AwaitingKeybind => app.theme.warning,
        InputMode::SettingKeybind => app.theme.accent,
        InputMode::EditingCps => app.theme.primary,
        _ => app.theme.secondary,
    };

    let instruction_text = match app.input_mode {
        InputMode::AwaitingKeybind => "🕐 Preparing to capture hotkey combination...",
        InputMode::SettingKeybind => {
            "⌨️  Press key combination (Ctrl+Shift+B, F1-F12, etc.) │ Esc=Cancel"
        }
        InputMode::EditingCps => "✏️  Enter CPS value (1-1000) │ Enter=Save │ Esc=Cancel",
        _ => {
            "🎮 ↑↓=Select │ Tab=Button │ E=Custom │ S=Hotkey │ M=Audio │ H=Hide │ R=Reset │ ?=Help │ Q=Quit"
        }
    };

    let instructions = Paragraph::new(vec![
        Spans::from(Span::styled(
            instruction_text,
            Style::default().fg(instruction_color),
        )),
        Spans::from(vec![
            Span::styled(
                "🏆 Pro Features: ",
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Global hotkeys • System tray • Audio feedback • Statistics • Auto-save"),
        ]),
    ])
    .style(Style::default().fg(app.theme.text))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" 🎛️  Controls & Information ")
            .border_style(Style::default().fg(app.theme.secondary)),
    );

    f.render_widget(instructions, chunks[4]);
}
