//! The app's text in every supported language. The language follows Windows'
//! display language unless the user picks one in the footer.
//!
//! Placeholders: `{n}` and `{level}` (numbers), `{name}`, `{when}`, `{email}`,
//! `{price}`. Phrases
//! with a count are worded so they read correctly for any number, because
//! several of these languages have more than two plural forms.
//!
//! Right-to-left and complex-script languages (Arabic, Hebrew, Hindi, Thai)
//! are left out: egui cannot shape them.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    En,
    De,
    Fr,
    Es,
    It,
    Pt,
    Nl,
    Pl,
    Tr,
    Ru,
    Ja,
    Ko,
    ZhHans,
    ZhHant,
}

impl Lang {
    pub const ALL: [Lang; 14] = [
        Lang::En,
        Lang::De,
        Lang::Fr,
        Lang::Es,
        Lang::It,
        Lang::Pt,
        Lang::Nl,
        Lang::Pl,
        Lang::Tr,
        Lang::Ru,
        Lang::Ja,
        Lang::Ko,
        Lang::ZhHans,
        Lang::ZhHant,
    ];

    /// Name for the language picker. East Asian names are in English because
    /// their glyphs are only loaded once that language is in use.
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::De => "Deutsch",
            Lang::Fr => "Français",
            Lang::Es => "Español",
            Lang::It => "Italiano",
            Lang::Pt => "Português",
            Lang::Nl => "Nederlands",
            Lang::Pl => "Polski",
            Lang::Tr => "Türkçe",
            Lang::Ru => "Русский",
            Lang::Ja => "Japanese",
            Lang::Ko => "Korean",
            Lang::ZhHans => "Chinese (Simplified)",
            Lang::ZhHant => "Chinese (Traditional)",
        }
    }

    /// Windows font files that cover this language, best first, each with
    /// how far to shift its glyphs down (as a fraction of the font size) so
    /// they share a baseline with Segoe UI. Empty means egui's
    /// built-in fonts are enough.
    pub fn system_fonts(self) -> &'static [(&'static str, f32)] {
        match self {
            Lang::Ja => &[("YuGothM.ttc", 0.34), ("meiryo.ttc", 0.34), ("msgothic.ttc", 0.16)],
            Lang::Ko => &[("malgun.ttf", 0.16), ("gulim.ttc", 0.16)],
            Lang::ZhHans => &[("msyh.ttc", 0.10), ("simsun.ttc", 0.16)],
            Lang::ZhHant => &[("msjh.ttc", 0.10), ("mingliu.ttc", 0.16)],
            _ => &[],
        }
    }

    /// Maps a Windows locale name such as `pt-BR` or `zh-Hant-TW`.
    pub fn from_locale(locale: &str) -> Lang {
        let lower = locale.to_ascii_lowercase();
        let mut parts = lower.split(['-', '_']);
        let language = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        match language {
            "de" => Lang::De,
            "fr" => Lang::Fr,
            "es" => Lang::Es,
            "it" => Lang::It,
            "pt" => Lang::Pt,
            "nl" => Lang::Nl,
            "pl" => Lang::Pl,
            "tr" => Lang::Tr,
            "ru" => Lang::Ru,
            "ja" => Lang::Ja,
            "ko" => Lang::Ko,
            "zh" => {
                if rest.iter().any(|p| ["hant", "tw", "hk", "mo"].contains(p)) {
                    Lang::ZhHant
                } else {
                    Lang::ZhHans
                }
            }
            _ => Lang::En,
        }
    }

    /// The language Windows is displayed in.
    pub fn system() -> Lang {
        use windows::Win32::Globalization::GetUserDefaultLocaleName;
        let mut buffer = [0u16; 85];
        let len = unsafe { GetUserDefaultLocaleName(&mut buffer) };
        if len <= 1 {
            return Lang::En;
        }
        Lang::from_locale(&String::from_utf16_lossy(&buffer[..len as usize - 1]))
    }

    pub fn strings(self) -> &'static Strings {
        match self {
            Lang::En => &EN,
            Lang::De => &DE,
            Lang::Fr => &FR,
            Lang::Es => &ES,
            Lang::It => &IT,
            Lang::Pt => &PT,
            Lang::Nl => &NL,
            Lang::Pl => &PL,
            Lang::Tr => &TR,
            Lang::Ru => &RU,
            Lang::Ja => &JA,
            Lang::Ko => &KO,
            Lang::ZhHans => &ZH_HANS,
            Lang::ZhHant => &ZH_HANT,
        }
    }
}

/// Fills `{key}` placeholders.
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut text = template.to_owned();
    for (key, value) in values {
        text = text.replace(&format!("{{{key}}}"), value);
    }
    text
}

/// Declares `Strings` and, for the tests, a list of every text in it.
macro_rules! strings {
    ($($(#[$doc:meta])* $field:ident,)*) => {
        pub struct Strings {
            $($(#[$doc])* pub $field: &'static str,)*
        }

        impl Strings {
            /// Every text, for tests.
            #[cfg(test)]
            fn all(&self) -> Vec<&'static str> {
                vec![$(self.$field),*]
            }
        }
    };
}

strings! {
    devices_title,
    refresh,
    unlock,
    /// `{email}`
    unlocked_for,
    searching,
    no_devices,
    kind_headphones,
    kind_mouse,
    kind_keyboard,
    kind_gamepad,
    kind_other,
    connected,
    off,
    /// `{level}`, `{when}`; follows `off`
    off_last,
    /// Follows `off`
    never_seen,
    no_report,
    unreadable,
    ago_now,
    /// `{n}`
    ago_minutes,
    /// `{n}`
    ago_hours,
    /// `{n}`
    ago_days,
    tray_choice,
    /// The same choice while only that device gets warnings.
    tray_choice_free,
    low_title,
    /// `{name}`, `{level}`
    low_body,
    /// Followed by a percentage.
    warn_at,
    autostart,
    /// What is free and what the unlock adds.
    plan,
    unlock_title,
    /// `{price}`
    buy,
    paste_key,
    activate,
    /// `{email}`
    unlocked_thanks,
    bad_key,
    auto_update,
    auto_update_hint,
    language,
    tray_open,
    tray_quit,
    tray_none,
    /// A device that is off, in the tooltip: "Mouse: off".
    off_short,
}

static EN: Strings = Strings {
    devices_title: "Bluetooth devices",
    refresh: "Refresh",
    unlock: "Unlock",
    unlocked_for: "Unlocked for {email}",
    searching: "Looking for Bluetooth devices…",
    no_devices: "No paired Bluetooth devices. Pair one in Windows Settings and it will appear here.",
    kind_headphones: "Audio",
    kind_mouse: "Mouse",
    kind_keyboard: "Keyboard",
    kind_gamepad: "Controller",
    kind_other: "Device",
    connected: "Connected",
    off: "Off or out of range",
    off_last: "last seen at {level}%, {when}",
    never_seen: "no battery level seen yet",
    no_report: "This device does not report its battery level",
    unreadable: "Couldn't read the battery level",
    ago_now: "just now",
    ago_minutes: "{n} min ago",
    ago_hours: "{n} h ago",
    ago_days: "{n} d ago",
    tray_choice: "Tray icon",
    tray_choice_free: "Tray icon and warning",
    low_title: "Low battery",
    low_body: "{name} is at {level}%",
    warn_at: "Warn when a battery falls to",
    autostart: "Start with Windows",
    plan: "Free: the battery level of every device, plus the tray icon and low-battery warning for one device you choose. A one-off unlock adds low-battery warnings for every device.",
    unlock_title: "Warnings for every device",
    buy: "Buy the unlock ({price})…",
    paste_key: "Already bought it? Paste your licence key:",
    activate: "Activate",
    unlocked_thanks: "Unlocked for {email}. Thank you!",
    bad_key: "That key isn't valid. Copy the whole key, starting with BTT1.",
    auto_update: "Automatically update",
    auto_update_hint: "Checks for new versions about once a day and installs them in the background.",
    language: "Language",
    tray_open: "Open Battery Tray",
    tray_quit: "Quit",
    tray_none: "No Bluetooth devices",
    off_short: "off",
};

static DE: Strings = Strings {
    devices_title: "Bluetooth-Geräte",
    refresh: "Aktualisieren",
    unlock: "Freischalten",
    unlocked_for: "Freigeschaltet für {email}",
    searching: "Bluetooth-Geräte werden gesucht…",
    no_devices: "Keine gekoppelten Bluetooth-Geräte. Koppeln Sie eines in den Windows-Einstellungen, dann erscheint es hier.",
    kind_headphones: "Audio",
    kind_mouse: "Maus",
    kind_keyboard: "Tastatur",
    kind_gamepad: "Controller",
    kind_other: "Gerät",
    connected: "Verbunden",
    off: "Aus oder außer Reichweite",
    off_last: "zuletzt {level} %, {when}",
    never_seen: "noch kein Akkustand gesehen",
    no_report: "Dieses Gerät meldet seinen Akkustand nicht",
    unreadable: "Akkustand konnte nicht gelesen werden",
    ago_now: "gerade eben",
    ago_minutes: "vor {n} Min.",
    ago_hours: "vor {n} Std.",
    ago_days: "vor {n} T.",
    tray_choice: "Infobereich-Symbol",
    tray_choice_free: "Infobereich-Symbol und Warnung",
    low_title: "Akku fast leer",
    low_body: "{name} ist bei {level} %",
    warn_at: "Warnen, wenn ein Akku fällt auf",
    autostart: "Mit Windows starten",
    plan: "Kostenlos: der Akkustand jedes Geräts sowie das Infobereich-Symbol und die Akkuwarnung für ein Gerät Ihrer Wahl. Eine einmalige Freischaltung ergänzt Akkuwarnungen für alle Geräte.",
    unlock_title: "Warnungen für alle Geräte",
    buy: "Freischaltung kaufen ({price})…",
    paste_key: "Schon gekauft? Lizenzschlüssel einfügen:",
    activate: "Aktivieren",
    unlocked_thanks: "Freigeschaltet für {email}. Vielen Dank!",
    bad_key: "Dieser Schlüssel ist ungültig. Kopieren Sie den ganzen Schlüssel, beginnend mit BTT1.",
    auto_update: "Automatisch aktualisieren",
    auto_update_hint: "Sucht etwa einmal täglich nach neuen Versionen und installiert sie im Hintergrund.",
    language: "Sprache",
    tray_open: "Battery Tray öffnen",
    tray_quit: "Beenden",
    tray_none: "Keine Bluetooth-Geräte",
    off_short: "aus",
};

static FR: Strings = Strings {
    devices_title: "Appareils Bluetooth",
    refresh: "Actualiser",
    unlock: "Débloquer",
    unlocked_for: "Débloqué pour {email}",
    searching: "Recherche des appareils Bluetooth…",
    no_devices: "Aucun appareil Bluetooth couplé. Couplez-en un dans les Paramètres Windows et il apparaîtra ici.",
    kind_headphones: "Audio",
    kind_mouse: "Souris",
    kind_keyboard: "Clavier",
    kind_gamepad: "Manette",
    kind_other: "Appareil",
    connected: "Connecté",
    off: "Éteint ou hors de portée",
    off_last: "dernier niveau {level} %, {when}",
    never_seen: "aucun niveau de batterie relevé pour l'instant",
    no_report: "Cet appareil n'indique pas son niveau de batterie",
    unreadable: "Impossible de lire le niveau de batterie",
    ago_now: "à l'instant",
    ago_minutes: "il y a {n} min",
    ago_hours: "il y a {n} h",
    ago_days: "il y a {n} j",
    tray_choice: "Icône de la zone de notification",
    tray_choice_free: "Icône et alerte",
    low_title: "Batterie faible",
    low_body: "{name} est à {level} %",
    warn_at: "Alerter quand une batterie descend à",
    autostart: "Démarrer avec Windows",
    plan: "Gratuit : le niveau de batterie de chaque appareil, plus l'icône de la zone de notification et l'alerte de batterie faible pour un appareil de votre choix. Un déblocage unique ajoute les alertes de batterie faible pour tous les appareils.",
    unlock_title: "Des alertes pour tous les appareils",
    buy: "Acheter le déblocage ({price})…",
    paste_key: "Déjà acheté ? Collez votre clé de licence :",
    activate: "Activer",
    unlocked_thanks: "Débloqué pour {email}. Merci !",
    bad_key: "Cette clé n'est pas valide. Copiez la clé entière, à partir de BTT1.",
    auto_update: "Mettre à jour automatiquement",
    auto_update_hint: "Recherche les nouvelles versions environ une fois par jour et les installe en arrière-plan.",
    language: "Langue",
    tray_open: "Ouvrir Battery Tray",
    tray_quit: "Quitter",
    tray_none: "Aucun appareil Bluetooth",
    off_short: "éteint",
};

static ES: Strings = Strings {
    devices_title: "Dispositivos Bluetooth",
    refresh: "Actualizar",
    unlock: "Desbloquear",
    unlocked_for: "Desbloqueado para {email}",
    searching: "Buscando dispositivos Bluetooth…",
    no_devices: "No hay dispositivos Bluetooth emparejados. Empareja uno en la Configuración de Windows y aparecerá aquí.",
    kind_headphones: "Audio",
    kind_mouse: "Ratón",
    kind_keyboard: "Teclado",
    kind_gamepad: "Mando",
    kind_other: "Dispositivo",
    connected: "Conectado",
    off: "Apagado o fuera de alcance",
    off_last: "último nivel {level} %, {when}",
    never_seen: "aún no se ha visto ningún nivel de batería",
    no_report: "Este dispositivo no informa de su nivel de batería",
    unreadable: "No se pudo leer el nivel de batería",
    ago_now: "ahora mismo",
    ago_minutes: "hace {n} min",
    ago_hours: "hace {n} h",
    ago_days: "hace {n} d",
    tray_choice: "Icono de la bandeja",
    tray_choice_free: "Icono de la bandeja y aviso",
    low_title: "Batería baja",
    low_body: "{name} está al {level} %",
    warn_at: "Avisar cuando una batería baje al",
    autostart: "Iniciar con Windows",
    plan: "Gratis: el nivel de batería de todos los dispositivos, más el icono de la bandeja y el aviso de batería baja para un dispositivo que elijas. Un desbloqueo de pago único añade avisos de batería baja para todos los dispositivos.",
    unlock_title: "Avisos para todos los dispositivos",
    buy: "Comprar el desbloqueo ({price})…",
    paste_key: "¿Ya lo compraste? Pega tu clave de licencia:",
    activate: "Activar",
    unlocked_thanks: "Desbloqueado para {email}. ¡Gracias!",
    bad_key: "Esa clave no es válida. Copia la clave completa, empezando por BTT1.",
    auto_update: "Actualizar automáticamente",
    auto_update_hint: "Busca versiones nuevas más o menos una vez al día y las instala en segundo plano.",
    language: "Idioma",
    tray_open: "Abrir Battery Tray",
    tray_quit: "Salir",
    tray_none: "No hay dispositivos Bluetooth",
    off_short: "apagado",
};

static IT: Strings = Strings {
    devices_title: "Dispositivi Bluetooth",
    refresh: "Aggiorna",
    unlock: "Sblocca",
    unlocked_for: "Sbloccato per {email}",
    searching: "Ricerca dei dispositivi Bluetooth…",
    no_devices: "Nessun dispositivo Bluetooth associato. Associane uno nelle Impostazioni di Windows e comparirà qui.",
    kind_headphones: "Audio",
    kind_mouse: "Mouse",
    kind_keyboard: "Tastiera",
    kind_gamepad: "Controller",
    kind_other: "Dispositivo",
    connected: "Connesso",
    off: "Spento o fuori portata",
    off_last: "ultimo livello {level}%, {when}",
    never_seen: "nessun livello della batteria rilevato finora",
    no_report: "Questo dispositivo non comunica il livello della batteria",
    unreadable: "Impossibile leggere il livello della batteria",
    ago_now: "proprio ora",
    ago_minutes: "{n} min fa",
    ago_hours: "{n} h fa",
    ago_days: "{n} g fa",
    tray_choice: "Icona nell'area di notifica",
    tray_choice_free: "Icona e avviso",
    low_title: "Batteria scarica",
    low_body: "{name} è al {level}%",
    warn_at: "Avvisa quando una batteria scende al",
    autostart: "Avvia con Windows",
    plan: "Gratis: il livello della batteria di ogni dispositivo, più l'icona nell'area di notifica e l'avviso di batteria scarica per un dispositivo a tua scelta. Uno sblocco una tantum aggiunge gli avvisi di batteria scarica per tutti i dispositivi.",
    unlock_title: "Avvisi per tutti i dispositivi",
    buy: "Acquista lo sblocco ({price})…",
    paste_key: "Già acquistato? Incolla la chiave di licenza:",
    activate: "Attiva",
    unlocked_thanks: "Sbloccato per {email}. Grazie!",
    bad_key: "Questa chiave non è valida. Copia la chiave intera, a partire da BTT1.",
    auto_update: "Aggiorna automaticamente",
    auto_update_hint: "Cerca nuove versioni circa una volta al giorno e le installa in background.",
    language: "Lingua",
    tray_open: "Apri Battery Tray",
    tray_quit: "Esci",
    tray_none: "Nessun dispositivo Bluetooth",
    off_short: "spento",
};

static PT: Strings = Strings {
    devices_title: "Dispositivos Bluetooth",
    refresh: "Atualizar",
    unlock: "Desbloquear",
    unlocked_for: "Desbloqueado para {email}",
    searching: "Procurando dispositivos Bluetooth…",
    no_devices: "Nenhum dispositivo Bluetooth emparelhado. Emparelhe um nas Configurações do Windows e ele aparecerá aqui.",
    kind_headphones: "Áudio",
    kind_mouse: "Mouse",
    kind_keyboard: "Teclado",
    kind_gamepad: "Controle",
    kind_other: "Dispositivo",
    connected: "Conectado",
    off: "Desligado ou fora de alcance",
    off_last: "último nível {level}%, {when}",
    never_seen: "nenhum nível de bateria visto até agora",
    no_report: "Este dispositivo não informa o nível da bateria",
    unreadable: "Não foi possível ler o nível da bateria",
    ago_now: "agora mesmo",
    ago_minutes: "há {n} min",
    ago_hours: "há {n} h",
    ago_days: "há {n} d",
    tray_choice: "Ícone da bandeja",
    tray_choice_free: "Ícone da bandeja e aviso",
    low_title: "Bateria fraca",
    low_body: "{name} está em {level}%",
    warn_at: "Avisar quando uma bateria cair para",
    autostart: "Iniciar com o Windows",
    plan: "Grátis: o nível de bateria de todos os dispositivos, mais o ícone da bandeja e o aviso de bateria fraca para um dispositivo à sua escolha. Um desbloqueio de pagamento único adiciona avisos de bateria fraca para todos os dispositivos.",
    unlock_title: "Avisos para todos os dispositivos",
    buy: "Comprar o desbloqueio ({price})…",
    paste_key: "Já comprou? Cole sua chave de licença:",
    activate: "Ativar",
    unlocked_thanks: "Desbloqueado para {email}. Obrigado!",
    bad_key: "Essa chave não é válida. Copie a chave inteira, começando por BTT1.",
    auto_update: "Atualizar automaticamente",
    auto_update_hint: "Procura novas versões cerca de uma vez por dia e as instala em segundo plano.",
    language: "Idioma",
    tray_open: "Abrir o Battery Tray",
    tray_quit: "Sair",
    tray_none: "Nenhum dispositivo Bluetooth",
    off_short: "desligado",
};

static NL: Strings = Strings {
    devices_title: "Bluetooth-apparaten",
    refresh: "Vernieuwen",
    unlock: "Ontgrendelen",
    unlocked_for: "Ontgrendeld voor {email}",
    searching: "Bluetooth-apparaten zoeken…",
    no_devices: "Geen gekoppelde Bluetooth-apparaten. Koppel er een in Windows-instellingen en het verschijnt hier.",
    kind_headphones: "Audio",
    kind_mouse: "Muis",
    kind_keyboard: "Toetsenbord",
    kind_gamepad: "Controller",
    kind_other: "Apparaat",
    connected: "Verbonden",
    off: "Uit of buiten bereik",
    off_last: "laatst gezien op {level}%, {when}",
    never_seen: "nog geen batterijniveau gezien",
    no_report: "Dit apparaat geeft zijn batterijniveau niet door",
    unreadable: "Kan het batterijniveau niet lezen",
    ago_now: "zojuist",
    ago_minutes: "{n} min geleden",
    ago_hours: "{n} u geleden",
    ago_days: "{n} d geleden",
    tray_choice: "Systeemvakpictogram",
    tray_choice_free: "Systeemvakpictogram en waarschuwing",
    low_title: "Batterij bijna leeg",
    low_body: "{name} staat op {level}%",
    warn_at: "Waarschuwen als een batterij zakt tot",
    autostart: "Starten met Windows",
    plan: "Gratis: het batterijniveau van elk apparaat, plus het systeemvakpictogram en de waarschuwing bij een bijna lege batterij voor één apparaat naar keuze. Een eenmalige ontgrendeling voegt waarschuwingen voor alle apparaten toe.",
    unlock_title: "Waarschuwingen voor alle apparaten",
    buy: "Ontgrendeling kopen ({price})…",
    paste_key: "Al gekocht? Plak je licentiesleutel:",
    activate: "Activeren",
    unlocked_thanks: "Ontgrendeld voor {email}. Bedankt!",
    bad_key: "Die sleutel is niet geldig. Kopieer de hele sleutel, beginnend met BTT1.",
    auto_update: "Automatisch bijwerken",
    auto_update_hint: "Zoekt ongeveer één keer per dag naar nieuwe versies en installeert ze op de achtergrond.",
    language: "Taal",
    tray_open: "Battery Tray openen",
    tray_quit: "Afsluiten",
    tray_none: "Geen Bluetooth-apparaten",
    off_short: "uit",
};

static PL: Strings = Strings {
    devices_title: "Urządzenia Bluetooth",
    refresh: "Odśwież",
    unlock: "Odblokuj",
    unlocked_for: "Odblokowano dla {email}",
    searching: "Wyszukiwanie urządzeń Bluetooth…",
    no_devices: "Brak sparowanych urządzeń Bluetooth. Sparuj urządzenie w Ustawieniach systemu Windows, a pojawi się tutaj.",
    kind_headphones: "Audio",
    kind_mouse: "Mysz",
    kind_keyboard: "Klawiatura",
    kind_gamepad: "Kontroler",
    kind_other: "Urządzenie",
    connected: "Połączono",
    off: "Wyłączone lub poza zasięgiem",
    off_last: "ostatnio {level}%, {when}",
    never_seen: "nie odczytano jeszcze poziomu baterii",
    no_report: "To urządzenie nie podaje poziomu baterii",
    unreadable: "Nie udało się odczytać poziomu baterii",
    ago_now: "przed chwilą",
    ago_minutes: "{n} min temu",
    ago_hours: "{n} godz. temu",
    ago_days: "{n} d temu",
    tray_choice: "Ikona w zasobniku",
    tray_choice_free: "Ikona w zasobniku i ostrzeżenie",
    low_title: "Niski poziom baterii",
    low_body: "{name}: {level}% baterii",
    warn_at: "Ostrzegaj, gdy bateria spadnie do",
    autostart: "Uruchamiaj z systemem Windows",
    plan: "Za darmo: poziom baterii każdego urządzenia oraz ikona w zasobniku i ostrzeżenie o niskim poziomie baterii dla jednego wybranego urządzenia. Jednorazowe odblokowanie dodaje ostrzeżenia dla wszystkich urządzeń.",
    unlock_title: "Ostrzeżenia dla wszystkich urządzeń",
    buy: "Kup odblokowanie ({price})…",
    paste_key: "Masz już zakup? Wklej klucz licencyjny:",
    activate: "Aktywuj",
    unlocked_thanks: "Odblokowano dla {email}. Dziękujemy!",
    bad_key: "Ten klucz jest nieprawidłowy. Skopiuj cały klucz, zaczynając od BTT1.",
    auto_update: "Aktualizuj automatycznie",
    auto_update_hint: "Mniej więcej raz dziennie sprawdza nowe wersje i instaluje je w tle.",
    language: "Język",
    tray_open: "Otwórz Battery Tray",
    tray_quit: "Zakończ",
    tray_none: "Brak urządzeń Bluetooth",
    off_short: "wyłączone",
};

static TR: Strings = Strings {
    devices_title: "Bluetooth cihazları",
    refresh: "Yenile",
    unlock: "Kilidi aç",
    unlocked_for: "{email} için kilit açıldı",
    searching: "Bluetooth cihazları aranıyor…",
    no_devices: "Eşleştirilmiş Bluetooth cihazı yok. Windows Ayarları'nda bir cihaz eşleştirin, burada görünecektir.",
    kind_headphones: "Ses",
    kind_mouse: "Fare",
    kind_keyboard: "Klavye",
    kind_gamepad: "Oyun kumandası",
    kind_other: "Cihaz",
    connected: "Bağlı",
    off: "Kapalı veya kapsama alanı dışında",
    off_last: "son görülen düzey %{level}, {when}",
    never_seen: "henüz pil düzeyi görülmedi",
    no_report: "Bu cihaz pil düzeyini bildirmiyor",
    unreadable: "Pil düzeyi okunamadı",
    ago_now: "az önce",
    ago_minutes: "{n} dk önce",
    ago_hours: "{n} sa önce",
    ago_days: "{n} gün önce",
    tray_choice: "Tepsi simgesi",
    tray_choice_free: "Tepsi simgesi ve uyarı",
    low_title: "Pil düşük",
    low_body: "{name} pili %{level}",
    warn_at: "Pil şu düzeye düştüğünde uyar:",
    autostart: "Windows ile başlat",
    plan: "Ücretsiz: her cihazın pil düzeyi, ayrıca seçtiğiniz bir cihaz için tepsi simgesi ve düşük pil uyarısı. Tek seferlik kilit açma, tüm cihazlar için düşük pil uyarılarını ekler.",
    unlock_title: "Tüm cihazlar için uyarılar",
    buy: "Kilit açmayı satın al ({price})…",
    paste_key: "Zaten satın aldınız mı? Lisans anahtarınızı yapıştırın:",
    activate: "Etkinleştir",
    unlocked_thanks: "{email} için kilit açıldı. Teşekkürler!",
    bad_key: "Bu anahtar geçerli değil. BTT1 ile başlayan anahtarın tamamını kopyalayın.",
    auto_update: "Otomatik güncelle",
    auto_update_hint: "Yaklaşık günde bir kez yeni sürümleri denetler ve arka planda yükler.",
    language: "Dil",
    tray_open: "Battery Tray'i aç",
    tray_quit: "Çıkış",
    tray_none: "Bluetooth cihazı yok",
    off_short: "kapalı",
};

static RU: Strings = Strings {
    devices_title: "Устройства Bluetooth",
    refresh: "Обновить",
    unlock: "Разблокировать",
    unlocked_for: "Разблокировано для {email}",
    searching: "Поиск устройств Bluetooth…",
    no_devices: "Нет сопряжённых устройств Bluetooth. Выполните сопряжение в параметрах Windows, и устройство появится здесь.",
    kind_headphones: "Аудио",
    kind_mouse: "Мышь",
    kind_keyboard: "Клавиатура",
    kind_gamepad: "Геймпад",
    kind_other: "Устройство",
    connected: "Подключено",
    off: "Выключено или вне зоны действия",
    off_last: "последний заряд {level} %, {when}",
    never_seen: "уровень заряда ещё не был получен",
    no_report: "Это устройство не сообщает уровень заряда",
    unreadable: "Не удалось получить уровень заряда",
    ago_now: "только что",
    ago_minutes: "{n} мин назад",
    ago_hours: "{n} ч назад",
    ago_days: "{n} дн. назад",
    tray_choice: "Значок в трее",
    tray_choice_free: "Значок в трее и предупреждение",
    low_title: "Низкий заряд",
    low_body: "{name}: заряд {level} %",
    warn_at: "Предупреждать, когда заряд падает до",
    autostart: "Запускать вместе с Windows",
    plan: "Бесплатно: уровень заряда каждого устройства, а также значок в трее и предупреждение о низком заряде для одного выбранного устройства. Разовая разблокировка добавляет предупреждения о низком заряде для всех устройств.",
    unlock_title: "Предупреждения для всех устройств",
    buy: "Купить разблокировку ({price})…",
    paste_key: "Уже купили? Вставьте лицензионный ключ:",
    activate: "Активировать",
    unlocked_thanks: "Разблокировано для {email}. Спасибо!",
    bad_key: "Этот ключ недействителен. Скопируйте ключ целиком, начиная с BTT1.",
    auto_update: "Обновлять автоматически",
    auto_update_hint: "Примерно раз в день проверяет новые версии и устанавливает их в фоновом режиме.",
    language: "Язык",
    tray_open: "Открыть Battery Tray",
    tray_quit: "Выход",
    tray_none: "Нет устройств Bluetooth",
    off_short: "выкл.",
};

static JA: Strings = Strings {
    devices_title: "Bluetooth デバイス",
    refresh: "更新",
    unlock: "ロック解除",
    unlocked_for: "{email} でロック解除済み",
    searching: "Bluetooth デバイスを探しています…",
    no_devices: "ペアリング済みの Bluetooth デバイスがありません。Windows の設定でペアリングすると、ここに表示されます。",
    kind_headphones: "オーディオ",
    kind_mouse: "マウス",
    kind_keyboard: "キーボード",
    kind_gamepad: "コントローラー",
    kind_other: "デバイス",
    connected: "接続済み",
    off: "電源オフまたは圏外",
    off_last: "最後の残量 {level}%、{when}",
    never_seen: "バッテリー残量はまだ取得していません",
    no_report: "このデバイスはバッテリー残量を通知しません",
    unreadable: "バッテリー残量を読み取れませんでした",
    ago_now: "たった今",
    ago_minutes: "{n}分前",
    ago_hours: "{n}時間前",
    ago_days: "{n}日前",
    tray_choice: "トレイアイコン",
    tray_choice_free: "トレイアイコンと警告",
    low_title: "バッテリー残量低下",
    low_body: "{name} の残量は {level}% です",
    warn_at: "バッテリーがこの残量まで減ったら警告",
    autostart: "Windows の起動時に開始",
    plan: "無料: すべてのデバイスのバッテリー残量と、選んだ 1 台のトレイアイコンと残量低下の警告。買い切りのロック解除で、すべてのデバイスの残量低下を警告します。",
    unlock_title: "すべてのデバイスで警告",
    buy: "ロック解除を購入 ({price})…",
    paste_key: "購入済みの場合は、ライセンスキーを貼り付けてください:",
    activate: "有効にする",
    unlocked_thanks: "{email} でロック解除しました。ありがとうございます。",
    bad_key: "このキーは無効です。BTT1 で始まるキー全体をコピーしてください。",
    auto_update: "自動的に更新する",
    auto_update_hint: "1日に1回ほど新しいバージョンを確認し、バックグラウンドでインストールします。",
    language: "言語",
    tray_open: "Battery Tray を開く",
    tray_quit: "終了",
    tray_none: "Bluetooth デバイスなし",
    off_short: "オフ",
};

static KO: Strings = Strings {
    devices_title: "Bluetooth 장치",
    refresh: "새로 고침",
    unlock: "잠금 해제",
    unlocked_for: "{email} 계정으로 잠금 해제됨",
    searching: "Bluetooth 장치를 찾는 중…",
    no_devices: "페어링된 Bluetooth 장치가 없습니다. Windows 설정에서 장치를 페어링하면 여기에 표시됩니다.",
    kind_headphones: "오디오",
    kind_mouse: "마우스",
    kind_keyboard: "키보드",
    kind_gamepad: "컨트롤러",
    kind_other: "장치",
    connected: "연결됨",
    off: "꺼져 있거나 범위 밖",
    off_last: "마지막 잔량 {level}%, {when}",
    never_seen: "아직 확인된 배터리 잔량 없음",
    no_report: "이 장치는 배터리 잔량을 알려 주지 않습니다",
    unreadable: "배터리 잔량을 읽지 못했습니다",
    ago_now: "방금",
    ago_minutes: "{n}분 전",
    ago_hours: "{n}시간 전",
    ago_days: "{n}일 전",
    tray_choice: "트레이 아이콘",
    tray_choice_free: "트레이 아이콘 및 경고",
    low_title: "배터리 부족",
    low_body: "{name} 배터리 {level}%",
    warn_at: "배터리가 다음 잔량까지 떨어지면 경고",
    autostart: "Windows 시작 시 실행",
    plan: "무료: 모든 장치의 배터리 잔량, 그리고 선택한 장치 한 대의 트레이 아이콘과 배터리 부족 경고. 한 번만 결제하는 잠금 해제로 모든 장치의 배터리 부족 경고를 받을 수 있습니다.",
    unlock_title: "모든 장치에 대한 경고",
    buy: "잠금 해제 구매 ({price})…",
    paste_key: "이미 구매하셨나요? 라이선스 키를 붙여 넣으세요:",
    activate: "활성화",
    unlocked_thanks: "{email} 계정으로 잠금 해제되었습니다. 감사합니다!",
    bad_key: "유효하지 않은 키입니다. BTT1로 시작하는 키 전체를 복사하세요.",
    auto_update: "자동으로 업데이트",
    auto_update_hint: "하루에 한 번 정도 새 버전을 확인하고 백그라운드에서 설치합니다.",
    language: "언어",
    tray_open: "Battery Tray 열기",
    tray_quit: "종료",
    tray_none: "Bluetooth 장치 없음",
    off_short: "꺼짐",
};

static ZH_HANS: Strings = Strings {
    devices_title: "蓝牙设备",
    refresh: "刷新",
    unlock: "解锁",
    unlocked_for: "已为 {email} 解锁",
    searching: "正在查找蓝牙设备…",
    no_devices: "没有已配对的蓝牙设备。请在 Windows 设置中配对设备，它就会显示在这里。",
    kind_headphones: "音频",
    kind_mouse: "鼠标",
    kind_keyboard: "键盘",
    kind_gamepad: "手柄",
    kind_other: "设备",
    connected: "已连接",
    off: "已关闭或不在范围内",
    off_last: "上次电量 {level}%，{when}",
    never_seen: "尚未读取到电量",
    no_report: "此设备不报告电池电量",
    unreadable: "无法读取电池电量",
    ago_now: "刚刚",
    ago_minutes: "{n} 分钟前",
    ago_hours: "{n} 小时前",
    ago_days: "{n} 天前",
    tray_choice: "托盘图标",
    tray_choice_free: "托盘图标和提醒",
    low_title: "电量不足",
    low_body: "{name} 电量为 {level}%",
    warn_at: "电量降到此值时提醒",
    autostart: "随 Windows 启动",
    plan: "免费：显示每台设备的电量，并为你选择的一台设备提供托盘图标和低电量提醒。一次性解锁后，所有设备都有低电量提醒。",
    unlock_title: "为所有设备提供提醒",
    buy: "购买解锁（{price}）…",
    paste_key: "已经购买？请粘贴许可证密钥：",
    activate: "激活",
    unlocked_thanks: "已为 {email} 解锁。谢谢！",
    bad_key: "该密钥无效。请复制以 BTT1 开头的完整密钥。",
    auto_update: "自动更新",
    auto_update_hint: "大约每天检查一次新版本，并在后台安装。",
    language: "语言",
    tray_open: "打开 Battery Tray",
    tray_quit: "退出",
    tray_none: "没有蓝牙设备",
    off_short: "已关闭",
};

static ZH_HANT: Strings = Strings {
    devices_title: "藍牙裝置",
    refresh: "重新整理",
    unlock: "解鎖",
    unlocked_for: "已為 {email} 解鎖",
    searching: "正在尋找藍牙裝置…",
    no_devices: "沒有已配對的藍牙裝置。請在 Windows 設定中配對裝置，它就會顯示在這裡。",
    kind_headphones: "音訊",
    kind_mouse: "滑鼠",
    kind_keyboard: "鍵盤",
    kind_gamepad: "控制器",
    kind_other: "裝置",
    connected: "已連線",
    off: "已關閉或不在範圍內",
    off_last: "上次電量 {level}%，{when}",
    never_seen: "尚未讀取到電量",
    no_report: "此裝置不回報電池電量",
    unreadable: "無法讀取電池電量",
    ago_now: "剛剛",
    ago_minutes: "{n} 分鐘前",
    ago_hours: "{n} 小時前",
    ago_days: "{n} 天前",
    tray_choice: "系統匣圖示",
    tray_choice_free: "系統匣圖示和提醒",
    low_title: "電量不足",
    low_body: "{name} 電量為 {level}%",
    warn_at: "電量降到此值時提醒",
    autostart: "隨 Windows 啟動",
    plan: "免費：顯示每部裝置的電量，並為你選擇的一部裝置提供系統匣圖示和低電量提醒。一次性解鎖後，所有裝置都有低電量提醒。",
    unlock_title: "為所有裝置提供提醒",
    buy: "購買解鎖（{price}）…",
    paste_key: "已經購買？請貼上授權金鑰：",
    activate: "啟用",
    unlocked_thanks: "已為 {email} 解鎖。謝謝！",
    bad_key: "此金鑰無效。請複製以 BTT1 開頭的完整金鑰。",
    auto_update: "自動更新",
    auto_update_hint: "大約每天檢查一次新版本，並在背景安裝。",
    language: "語言",
    tray_open: "開啟 Battery Tray",
    tray_quit: "結束",
    tray_none: "沒有藍牙裝置",
    off_short: "已關閉",
};

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(text: &str) -> Vec<String> {
        let mut found: Vec<String> = text
            .match_indices('{')
            .filter_map(|(start, _)| {
                let end = text[start..].find('}')?;
                Some(text[start..start + end + 1].to_owned())
            })
            .collect();
        found.sort();
        found
    }

    #[test]
    fn every_language_keeps_the_english_placeholders_and_has_no_empty_text() {
        let english = EN.all();
        for lang in Lang::ALL {
            let texts = lang.strings().all();
            assert_eq!(texts.len(), english.len());
            for (index, text) in texts.iter().enumerate() {
                assert!(!text.trim().is_empty(), "{lang:?} text {index} is empty");
                assert_eq!(
                    placeholders(text),
                    placeholders(english[index]),
                    "{lang:?} text {index}: {text}"
                );
            }
        }
    }

    #[test]
    fn every_language_states_the_free_and_paid_rule_and_fits_the_tray() {
        for lang in Lang::ALL {
            let t = lang.strings();
            // The plan text says what is free and what the unlock adds; it
            // must not be a copy of the English.
            assert!(t.plan.chars().count() > 40, "{lang:?}");
            if lang != Lang::En {
                assert_ne!(t.plan, EN.plan, "{lang:?}");
            }
            // Windows cuts a notification title at 63 UTF-16 units.
            assert!(t.low_title.encode_utf16().count() <= 63, "{lang:?}");
        }
    }

    #[test]
    fn locales_map_to_languages() {
        for (locale, lang) in [
            ("en-GB", Lang::En),
            ("de-AT", Lang::De),
            ("pt-BR", Lang::Pt),
            ("zh-CN", Lang::ZhHans),
            ("zh-Hans-SG", Lang::ZhHans),
            ("zh-TW", Lang::ZhHant),
            ("zh-Hant-HK", Lang::ZhHant),
            ("ja-JP", Lang::Ja),
            ("ru-RU", Lang::Ru),
            ("sv-SE", Lang::En),
            ("", Lang::En),
        ] {
            assert_eq!(Lang::from_locale(locale), lang, "{locale}");
        }
    }

    #[test]
    fn fill_replaces_each_placeholder() {
        assert_eq!(
            fill("{name} is at {level}%", &[("name", "Mouse"), ("level", "15")]),
            "Mouse is at 15%"
        );
    }
}
