use looper_core::action::Action;
use looper_core::config::AppConfig;
use looper_core::looper::LooperState;
use looper_core::settings::SettingsState;
use looper_hal::Backends;

/// Which screen is up, and that screen's state - plain data, no `egui::`
/// types.
/// Both boxed: an enum is as big as its biggest variant, the two states
/// differ by hundreds of bytes, and a screen changes rarely enough that
/// the allocation is nothing.
pub enum Screen {
    Settings(Box<SettingsState>),
    Looper(Box<LooperState>),
}

pub struct App {
    pub screen: Screen,
    /// Every backend this build offers. Owned here because it outlives
    /// any one screen - both of them ask it what devices exist.
    pub backends: Backends,
}

impl App {
    /// Straight into the looper if a saved config still opens, otherwise
    /// Settings, carrying the failure so it can say why.
    pub fn new(backends: Backends) -> Self {
        let mut app = Self {
            screen: Screen::Settings(Box::new(SettingsState::new(None))),
            backends,
        };
        match AppConfig::load() {
            Some(config) => app.start_looper(config),
            None => app.open_settings(None),
        }
        app
    }

    pub fn apply(&mut self, action: Action) {
        match action {
            Action::Start(config) => self.start_looper(config),
            Action::OpenSettings => self.open_settings(None),
            // Only the looper screen asks; the screen it asked from may
            // have gone already, if Settings opened in the same frame.
            Action::Looper(action) => {
                if let Screen::Looper(looper) = &mut self.screen {
                    looper.act(action);
                }
            }
        }
    }

    /// Persists the choice only once the device is known to open; on
    /// failure Settings stays put and shows why.
    fn start_looper(&mut self, config: AppConfig) {
        match LooperState::start(&self.backends, &config) {
            Ok(looper) => {
                let _ = config.save();
                self.screen = Screen::Looper(Box::new(looper));
            }
            Err(err) => match &mut self.screen {
                Screen::Settings(settings) => settings.error = Some(err),
                // Nothing to fail back to yet, at startup.
                Screen::Looper(_) => self.open_settings(Some(err)),
            },
        }
    }

    /// Puts the settings screen up, *then* asks what devices there are.
    ///
    /// The order is load-bearing. ASIO only lets one driver be loaded at
    /// a time, and enumeration loads each in turn to ask its name - so
    /// with our own streams still open, cpal stops enumerating at the
    /// first driver it can't claim. Installing the screen first drops
    /// the looper, and the streams with it.
    fn open_settings(&mut self, error: Option<String>) {
        self.screen = Screen::Settings(Box::new(SettingsState::new(error)));
        if let Screen::Settings(settings) = &mut self.screen {
            settings.refresh(&self.backends);
        }
    }
}
