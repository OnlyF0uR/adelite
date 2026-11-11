//! The conversation with TDLib: commands out, what it says in, and signing in.

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use adelite_chat::{Command, Event, Field, List, Prompt, Session};
use serde_json::{Value, json};

use crate::incoming::{self, friendly};
use crate::map::{Cache, int, kind, string};
use crate::request::{self, Target, quiet};
use crate::seal;
use crate::tdjson::{Inbox, Library};

/// The file in the data folder holding the API id and hash.
const APP_FILE: &str = "app.json";
/// Events delivered at once at most, so a long sync still reaches the view in pieces.
const BATCH: usize = 256;

/// Which answer the prompt on screen is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Nothing is asked.
    None,
    /// The API id and hash.
    App,
    Phone,
    Code,
    Password,
    Email,
    EmailCode,
}

/// What the caller's thread and the receiving thread share.
pub struct Shared {
    library: Arc<Library>,
    /// The TDLib client in use; replaced after signing out, which closes one.
    client: AtomicI32,
    /// The prompt on screen and the step it belongs to.
    asking: Mutex<(Step, Prompt)>,
    /// What is remembered of users and chats. The receiving thread writes
    /// it; a command reads it for what its request depends on.
    cache: Mutex<Cache>,
    /// The app is going: the closed client is not replaced.
    closing: AtomicBool,
    /// Telegram refused the API id and hash: say so when asking again.
    refused: AtomicBool,
    dir: PathBuf,
    events: Sender<Event>,
    wake: Box<dyn Fn() + Send + Sync>,
}

/// Whether the receiving thread goes on.
#[derive(PartialEq, Eq)]
enum Flow {
    Continue,
    Stop,
}

impl Shared {
    pub fn new(library: Arc<Library>, dir: PathBuf, events: Sender<Event>, wake: Box<dyn Fn() + Send + Sync>) -> Self {
        // TDLib writes every request to stderr by default: errors only.
        let _ = library.execute(r#"{"@type":"setLogVerbosityLevel","new_verbosity_level":1}"#);
        Self {
            client: AtomicI32::new(library.client()),
            library,
            asking: Mutex::new((Step::None, Prompt::default())),
            cache: Mutex::new(Cache::default()),
            closing: AtomicBool::new(false),
            refused: AtomicBool::new(false),
            dir,
            events,
            wake,
        }
    }

    /// Sends `request` to the client in use.
    fn request(&self, request: &Value) {
        self.library.send(self.client.load(Ordering::Relaxed), &request.to_string());
    }

    /// What is remembered. Never held while waiting for TDLib.
    fn cache(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts the client: it wakes with its first request, then says what it needs.
    pub fn begin(&self) {
        self.request(&json!({"@type": "getOption", "name": "version", "@extra": quiet()}));
    }

    /// Hands one event to the view at once.
    pub fn report(&self, event: Event) {
        let _ = self.events.send(event);
        (self.wake)();
    }

    /// Asks TDLib to close: it writes what it holds, then says it has.
    pub fn close(&self) {
        self.closing.store(true, Ordering::Relaxed);
        self.request(&json!({"@type": "close"}));
    }

    /// Shows `prompt` and remembers which answer it waits for.
    fn ask(&self, step: Step, prompt: Prompt, out: &mut Vec<Event>) {
        *self.asking.lock().unwrap_or_else(PoisonError::into_inner) = (step, prompt.clone());
        out.push(Event::Session(Session::Asking(prompt)));
    }

    /// Carries out a command of the view's.
    pub fn command(&self, command: Command) {
        match command {
            Command::Answer(values) => self.answer(&values),
            Command::Back => {
                let mut out = Vec::new();
                self.ask(Step::Phone, phone_prompt(), &mut out);
                for event in out {
                    self.report(event);
                }
            }
            // The code, sent the next way Telegram has for it. It refuses before its wait is over.
            Command::Alternative => self.request(&json!({
                "@type": "resendAuthenticationCode",
                "reason": {"@type": "resendCodeReasonUserRequest"},
                "@extra": {"for": "auth"},
            })),
            Command::LogOut => self.request(&json!({"@type": "logOut"})),
            Command::Follow(address) => match request::target(&address) {
                // A page outside is none of TDLib's business.
                Target::Outside(url) => self.report(Event::Browse(url)),
                _ => self.carry_out(&self.cache(), Command::Follow(address)),
            },
            command => self.carry_out(&self.cache(), command),
        }
    }

    /// Sends the requests a command comes to.
    fn carry_out(&self, cache: &Cache, command: Command) {
        for request in request::requests(command, cache, &|text| self.marked_up(text)) {
            self.request(&request);
        }
    }

    /// `text` as a `formattedText`, its Markdown (`**bold**`, `` `code` ``)
    /// read by TDLib; as it is when TDLib will not read it.
    fn marked_up(&self, text: &str) -> Value {
        let plain = json!({"@type": "formattedText", "text": text});
        let parsed = self.library.execute(&json!({"@type": "parseMarkdown", "text": plain}).to_string());
        match parsed.and_then(|answer| serde_json::from_str::<Value>(&answer).ok()) {
            Some(parsed) if kind(&parsed) == "formattedText" => parsed,
            _ => plain,
        }
    }

    /// Passes the user's answer to the prompt on to TDLib.
    fn answer(&self, values: &[String]) {
        let (step, prompt) = self.asking.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let value = |index: usize| values.get(index).map_or("", |value| value.trim());
        let request = match step {
            Step::None => return,
            Step::App => {
                let (id, hash) = (value(0).parse::<i32>().ok().filter(|id| *id > 0), value(1));
                let hash_fits = hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit());
                let (Some(id), true) = (id, hash_fits) else {
                    let problem = "The API id is a number and the API hash 32 letters and digits.".to_owned();
                    return self.report(Event::Session(Session::Asking(Prompt { problem, ..prompt })));
                };
                if let Err(e) = save_app(&self.dir, id, hash) {
                    self.report(Event::Problem(format!("The API id and hash could not be saved: {e}")));
                }
                parameters(&self.dir, id, hash)
            }
            Step::Phone => match phone_number(value(0)) {
                Ok(number) => json!({"@type": "setAuthenticationPhoneNumber", "phone_number": number}),
                Err(problem) => {
                    let problem = problem.to_owned();
                    return self.report(Event::Session(Session::Asking(Prompt { problem, ..prompt })));
                }
            },
            Step::Code => json!({"@type": "checkAuthenticationCode", "code": value(0)}),
            Step::Password => json!({"@type": "checkAuthenticationPassword", "password": values.first()}),
            Step::Email => json!({"@type": "setAuthenticationEmailAddress", "email_address": value(0)}),
            Step::EmailCode => json!({
                "@type": "checkAuthenticationEmailCode",
                "code": {"@type": "emailAddressAuthenticationCode", "code": value(0)},
            }),
        };
        let mut request = request;
        request["@extra"] = json!({"for": "auth"});
        self.request(&request);
    }

    /// The receiving thread: maps what TDLib says until it has closed.
    pub fn run(&self, mut inbox: Inbox) {
        let mut out = Vec::new();
        let mut silent = 0;
        loop {
            // With events in hand, only take what is already there, then deliver.
            let timeout = if out.is_empty() { 5.0 } else { 0.0 };
            let update = inbox.receive(timeout, |text| serde_json::from_str::<Value>(text).ok());
            let idle = update.is_none();
            let flow = match update {
                Some(Some(update)) => {
                    silent = 0;
                    let mut cache = self.cache();
                    self.handle(&mut cache, &update, &mut out)
                }
                Some(None) => Flow::Continue,
                None => {
                    silent += 1;
                    // Asked to close and not a word for ten seconds: it will not come.
                    if self.closing.load(Ordering::Relaxed) && silent >= 2 { Flow::Stop } else { Flow::Continue }
                }
            };
            if idle || out.len() >= BATCH || flow == Flow::Stop {
                self.deliver(&mut out);
            }
            if flow == Flow::Stop {
                return;
            }
        }
    }

    /// Sends the events gathered and wakes the interface once.
    fn deliver(&self, out: &mut Vec<Event>) {
        if out.is_empty() {
            return;
        }
        for event in out.drain(..) {
            let _ = self.events.send(event);
        }
        (self.wake)();
    }

    /// Maps one thing TDLib said into events. Signing in is followed here;
    /// the rest is `incoming`'s, which may ask for more in turn.
    fn handle(&self, cache: &mut Cache, said: &Value, out: &mut Vec<Event>) -> Flow {
        if kind(said) == "updateAuthorizationState" {
            let flow = self.authorization(cache, &said["authorization_state"], out);
            // Signed in: the lists are told, whatever Telegram goes on to say about them.
            out.extend(cache.lists());
            return flow;
        }
        if kind(said) == "error" && said["@extra"]["for"] == "auth" {
            self.refused(string(&said["message"]), out);
            return Flow::Continue;
        }
        let mut ask = Vec::new();
        incoming::take(cache, said, now(), out, &mut ask);
        for request in &ask {
            self.request(request);
        }
        Flow::Continue
    }

    /// Follows TDLib through signing in.
    fn authorization(&self, cache: &mut Cache, state: &Value, out: &mut Vec<Event>) -> Flow {
        let stuck = |title: &str, detail: &str| Prompt {
            title: title.to_owned(),
            detail: detail.to_owned(),
            back: true,
            ..Prompt::default()
        };
        match kind(state) {
            // The build's own API id; a developer's saved one in a build without; asked for last.
            "authorizationStateWaitTdlibParameters" => match built_in().or_else(|| load_app(&self.dir)) {
                Some((id, hash)) => self.request(&parameters(&self.dir, id, &hash)),
                None => self.ask(Step::App, app_prompt(self.refused.swap(false, Ordering::Relaxed)), out),
            },
            "authorizationStateWaitPhoneNumber" => self.ask(Step::Phone, phone_prompt(), out),
            "authorizationStateWaitCode" => self.ask(Step::Code, code_prompt(&state["code_info"]), out),
            "authorizationStateWaitPassword" => {
                self.ask(Step::Password, password_prompt(string(&state["password_hint"])), out);
            }
            "authorizationStateWaitEmailAddress" => self.ask(Step::Email, email_prompt(), out),
            "authorizationStateWaitEmailCode" => {
                let pattern = string(&state["code_info"]["email_address_pattern"]);
                self.ask(Step::EmailCode, email_code_prompt(pattern), out);
            }
            "authorizationStateWaitRegistration" => self.ask(
                Step::None,
                stuck(
                    "No Telegram account yet",
                    "This number has no Telegram account. Create one in Telegram's own app, then sign in here.",
                ),
                out,
            ),
            "authorizationStateWaitOtherDeviceConfirmation" | "authorizationStateWaitPremiumPurchase" => self.ask(
                Step::None,
                stuck("Telegram asks for something else", "This way of signing in is not supported here yet."),
                out,
            ),
            "authorizationStateReady" => {
                self.ask(Step::None, Prompt::default(), &mut Vec::new());
                out.push(Event::Session(Session::Ready));
                self.signed_in(cache);
            }
            "authorizationStateLoggingOut" | "authorizationStateClosing" => {
                cache.clear();
                out.push(Event::Session(Session::Starting));
            }
            "authorizationStateClosed" => {
                if self.closing.load(Ordering::Relaxed) {
                    return Flow::Stop;
                }
                // Signed out: a closed client is over, the next one signs in again.
                cache.clear();
                self.client.store(self.library.client(), Ordering::Relaxed);
                self.begin();
            }
            _ => {}
        }
        Flow::Continue
    }

    /// Signed in: asks for the chats, and for what TDLib tells only on asking.
    fn signed_in(&self, cache: &mut Cache) {
        cache.signed_in();
        self.carry_out(cache, Command::LoadChats(List::Main));
        // Whether a chat is muted may be left to the settings for its kind of chat.
        for scope in ["PrivateChats", "GroupChats", "ChannelChats"] {
            let scope = json!({"@type": format!("notificationSettingsScope{scope}")});
            self.request(&json!({
                "@type": "getScopeNotificationSettings", "scope": scope, "@extra": {"for": "scope", "scope": scope},
            }));
        }
        self.request(&json!({"@type": "getJoinedCommunities", "@extra": {"for": "communities"}}));
    }

    /// TDLib refused a step of signing in: what it means depends on what was asked.
    fn refused(&self, message: &str, out: &mut Vec<Event>) {
        if message.to_lowercase().contains("api_") && built_in().is_some() {
            // Nothing the user can put right: the app itself is not accepted.
            out.push(Event::Session(Session::Unavailable {
                title: "Telegram did not accept this app".to_owned(),
                detail: format!("The Telegram API id this copy of Adelite was built with was refused ({message})."),
            }));
        } else if message.contains("API_ID") {
            // The parameters cannot be changed in a running client: start over and ask again.
            let _ = fs::remove_file(self.dir.join(APP_FILE));
            self.refused.store(true, Ordering::Relaxed);
            self.request(&json!({"@type": "close"}));
        } else {
            let (step, prompt) = self.asking.lock().unwrap_or_else(PoisonError::into_inner).clone();
            match step {
                // Nothing was asked: TDLib refused the parameters it was started with.
                Step::None if message.to_lowercase().contains("api_") => {
                    // The saved API id and hash are no good: ask for them again.
                    let _ = fs::remove_file(self.dir.join(APP_FILE));
                    self.ask(Step::App, Prompt { problem: friendly(message), ..app_prompt(false) }, out);
                }
                Step::None => out.push(Event::Session(Session::Unavailable {
                    title: "Telegram could not be started".to_owned(),
                    detail: message.to_owned(),
                })),
                _ => out.push(Event::Session(Session::Asking(Prompt { problem: friendly(message), ..prompt }))),
            }
        }
    }
}

/// The time, in Unix seconds.
fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs() as i64)
}

/// The request that tells TDLib who the app is and where its data goes.
fn parameters(dir: &Path, id: i32, hash: &str) -> Value {
    json!({
        "@type": "setTdlibParameters",
        // Lossy rather than `Serialize`, which fails on a path that is not Unicode.
        "database_directory": dir.join("tdlib").to_string_lossy(),
        "files_directory": dir.join("files").to_string_lossy(),
        "use_file_database": true,
        "use_chat_info_database": true,
        "use_message_database": true,
        "use_secret_chats": false,
        "api_id": id,
        "api_hash": hash,
        "system_language_code": "en",
        "device_model": "Desktop",
        "application_version": env!("CARGO_PKG_VERSION"),
        "@extra": {"for": "auth"},
    })
}

/// What the build script made of the API id and hash it was given.
mod sealed {
    // Its consts name the sealed block's size by this path.
    include!(concat!(env!("OUT_DIR"), "/sealed.rs"));
}

/// The API id and hash the app was built with (`build.rs`). They name the
/// app to Telegram, not its user, so a build that has them never asks
/// anyone for them. They are kept sealed and opened here, each time TDLib
/// asks: the seeds pass through `black_box`, so the compiler cannot work
/// the answer out ahead of time and leave it in the binary as text.
fn built_in() -> Option<(i32, String)> {
    let (app, seeds) = sealed::APP?;
    seal::open(app, std::hint::black_box(seeds))
}

/// Whether this build carries an API id, for the tests.
#[cfg(test)]
pub fn has_built_in() -> bool {
    built_in().is_some()
}

/// The API id and hash a developer saved, if there are any that read.
fn load_app(dir: &Path) -> Option<(i32, String)> {
    let saved: Value = serde_json::from_slice(&fs::read(dir.join(APP_FILE)).ok()?).ok()?;
    let id = i32::try_from(saved["api_id"].as_i64()?).ok()?;
    Some((id, saved["api_hash"].as_str()?.to_owned()))
}

/// Saves the API id and hash, readable only by the user on Unix.
fn save_app(dir: &Path, id: i32, hash: &str) -> std::io::Result<()> {
    let mut folders = fs::DirBuilder::new();
    folders.recursive(true);
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        std::os::unix::fs::DirBuilderExt::mode(&mut folders, 0o700);
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    }
    folders.create(dir)?;
    // Written beside it and renamed: never half a file.
    let (path, tmp) = (dir.join(APP_FILE), dir.join(format!("{APP_FILE}.tmp")));
    let mut file = options.open(&tmp)?;
    file.write_all(json!({"api_id": id, "api_hash": hash}).to_string().as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, path)
}

/// A prompt with one field.
fn prompt(title: &str, detail: String, field: Field, back: bool) -> Prompt {
    Prompt {
        title: title.to_owned(),
        detail,
        fields: vec![field],
        action: "Continue".to_owned(),
        back,
        ..Prompt::default()
    }
}

fn field(label: &str, hint: &str, secret: bool) -> Field {
    Field { label: label.to_owned(), hint: hint.to_owned(), secret }
}

fn app_prompt(refused: bool) -> Prompt {
    Prompt {
        title: "Connect Telegram".to_owned(),
        detail: "This copy of Adelite was built without a Telegram API id. Create one under “API development \
                 tools” and paste it here to try it; a release is built with its own and never asks."
            .to_owned(),
        fields: vec![field("API id", "1234567", false), field("API hash", "32 letters and digits", true)],
        action: "Continue".to_owned(),
        link: Some(("my.telegram.org".to_owned(), "https://my.telegram.org/apps".to_owned())),
        problem: if refused { "Telegram did not accept that API id and hash.".to_owned() } else { String::new() },
        back: false,
        alternative: String::new(),
    }
}

/// A typed phone number as Telegram wants it: `+`, the country code, the
/// rest, digits only. TDLib itself only drops what is not a digit, so a
/// number typed the way people write them can reach Telegram as another
/// number altogether, and the code goes to a stranger or nowhere: a
/// national number (`06 1234 5678`) reads as a country code and a short
/// number, and the `(0)` in `+31 (0)6 …` as a digit of it. Those are put
/// right or refused here, with what to type instead.
fn phone_number(typed: &str) -> Result<String, &'static str> {
    const FORMAT: &str = "Start with + and your country code, like +31 6 12345678.";
    // The trunk zero in brackets is dialled inside the country only.
    let typed = typed.trim().replace("(0)", "");
    let international = match typed.strip_prefix("00") {
        Some(rest) => rest,
        None => typed.strip_prefix('+').ok_or(FORMAT)?,
    };
    let stray = |c: char| !(c.is_ascii_digit() || c.is_whitespace() || "-.()/".contains(c));
    if international.chars().any(stray) {
        return Err("A phone number is digits only, after the +.");
    }
    let digits: String = international.chars().filter(char::is_ascii_digit).collect();
    // No country code starts with a zero; a whole number is 7 to 15 digits (E.164).
    if digits.starts_with('0') {
        return Err(FORMAT);
    }
    if !(7..=15).contains(&digits.len()) {
        return Err("That is not a whole phone number: with its country code it has 7 to 15 digits.");
    }
    Ok(format!("+{digits}"))
}

fn phone_prompt() -> Prompt {
    let detail = "Your phone number in international form: +, the country code, then the number without its \
                  leading zero. Telegram sends a code to sign in with."
        .to_owned();
    prompt("Sign in to Telegram", detail, field("Phone number", "+31 6 12345678", false), false)
}

/// The prompt for the code, saying where Telegram sent it: that is
/// rarely an SMS. Where Telegram offers another way after a wait, the
/// prompt offers it too.
fn code_prompt(info: &Value) -> Prompt {
    // As Telegram has it, so a number that went in wrong shows.
    let number = string(&info["phone_number"]).trim_start_matches('+');
    let mut detail = match kind(&info["type"]) {
        "authenticationCodeTypeTelegramMessage" => format!(
            "Telegram sent the code for +{number} as a message inside Telegram, not by SMS. Open Telegram on a phone \
             or computer where that number is signed in and look in the chat called “Telegram”."
        ),
        "authenticationCodeTypeSms" | "authenticationCodeTypeSmsWord" | "authenticationCodeTypeSmsPhrase" => {
            format!("Telegram sent the code by SMS to +{number}.")
        }
        "authenticationCodeTypeCall" => format!("Telegram is calling +{number} and reads the code out."),
        "authenticationCodeTypeFlashCall" | "authenticationCodeTypeMissedCall" => {
            format!("Telegram is calling +{number} and hangs up: the code is the last digits of the number that calls.")
        }
        "authenticationCodeTypeFragment" => format!("The code for +{number} is waiting on fragment.com."),
        "authenticationCodeTypeFirebaseAndroid" | "authenticationCodeTypeFirebaseIos" => {
            format!(
                "Telegram wants to send the code for +{number} through a phone's own services, which a desktop app \
                 cannot receive."
            )
        }
        _ => format!("Telegram sent a code for +{number}."),
    };
    let alternative = match kind(&info["next_type"]) {
        "" => "",
        "authenticationCodeTypeSms" | "authenticationCodeTypeSmsWord" | "authenticationCodeTypeSmsPhrase" => {
            "Send the code by SMS instead"
        }
        "authenticationCodeTypeCall" | "authenticationCodeTypeFlashCall" | "authenticationCodeTypeMissedCall" => {
            "Call me with the code instead"
        }
        _ => "Send the code another way",
    };
    let wait = int(&info["timeout"]);
    if !alternative.is_empty() && wait > 0 {
        let wait = if wait >= 120 { format!("{} minutes", wait / 60) } else { format!("{wait} seconds") };
        let _ = write!(detail, " If it does not come, Telegram lets you ask for it another way after {wait}.");
    }
    Prompt {
        alternative: alternative.to_owned(),
        ..prompt("Enter the code", detail, field("Code", "12345", false), true)
    }
}

fn password_prompt(hint: &str) -> Prompt {
    let detail = if hint.is_empty() {
        "Your account has two-step verification: enter its password.".to_owned()
    } else {
        format!("Your account has two-step verification: enter its password. Hint: {hint}")
    };
    prompt("Enter your password", detail, field("Password", "", true), true)
}

fn email_prompt() -> Prompt {
    let detail = "Telegram wants an email address to send this account's sign-in codes to.".to_owned();
    prompt("Enter your email", detail, field("Email", "you@example.com", false), true)
}

fn email_code_prompt(pattern: &str) -> Prompt {
    prompt("Enter the code", format!("Telegram sent a code to {pattern}."), field("Code", "123456", false), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_numbers_reach_telegram_in_international_form() {
        for typed in ["+31612345678", " +31 6 1234 5678 ", "+31 (0)6 12345678", "0031 6-1234.5678", "+31(6)12345678"] {
            assert_eq!(phone_number(typed).as_deref(), Ok("+31612345678"), "{typed:?}");
        }
        assert_eq!(phone_number("+1 (415) 555-0134").as_deref(), Ok("+14155550134"));
        // What would reach Telegram as another number is refused, not sent.
        for typed in ["0612345678", "06 1234 5678", "612345678", "4155550134", "+0612345678", "000612345678", ""] {
            assert!(phone_number(typed).is_err_and(|problem| problem.contains("country code")), "{typed:?}");
        }
        for typed in ["+31 6 1234 567x", "+31612345678 ext 4", "+٣١٦١٢٣٤٥٦٧٨"] {
            assert!(phone_number(typed).is_err_and(|problem| problem.contains("digits only")), "{typed:?}");
        }
        for typed in ["+31612", "+3161234567890123", "+"] {
            assert!(phone_number(typed).is_err_and(|problem| problem.contains("7 to 15")), "{typed:?}");
        }
    }

    #[test]
    fn the_api_id_and_hash_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("adelite-telegram-test-{}", std::process::id()));
        assert_eq!(load_app(&dir), None);
        save_app(&dir, 1_234_567, "0123456789abcdef0123456789abcdef").expect("saved");
        assert_eq!(load_app(&dir), Some((1_234_567, "0123456789abcdef0123456789abcdef".to_owned())));
        let request = parameters(&dir, 1_234_567, "0123456789abcdef0123456789abcdef");
        assert_eq!((request["api_id"].as_i64(), request["use_secret_chats"].as_bool()), (Some(1_234_567), Some(false)));
        fs::write(dir.join(APP_FILE), b"{\"api_id\": \"no\"}").expect("written");
        assert_eq!(load_app(&dir), None, "a file that does not read asks again");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn prompts_ask_for_what_telegram_waits_for() {
        let code = code_prompt(
            &serde_json::json!({"phone_number": "31612345678", "type": {"@type": "authenticationCodeTypeSms"}}),
        );
        assert!(code.detail.contains("+31612345678") && code.back && code.fields.len() == 1);
        assert!(code.alternative.is_empty(), "no other way is offered where Telegram has none");
        // Sent inside Telegram, with an SMS to follow: the prompt says where to look, and offers the SMS.
        let inside = code_prompt(&serde_json::json!({"phone_number": "31612345678", "timeout": 60,
            "type": {"@type": "authenticationCodeTypeTelegramMessage"}, "next_type": {"@type": "authenticationCodeTypeSms"}}));
        assert!(inside.detail.contains("not by SMS") && inside.detail.ends_with("after 60 seconds."));
        assert_eq!(inside.alternative, "Send the code by SMS instead");
        let unknown = code_prompt(&serde_json::json!({"type": {"@type": "authenticationCodeTypeNew"}}));
        assert_eq!(unknown.detail, "Telegram sent a code for +.", "a way this app does not know is not called an SMS");
        assert!(inside.detail.contains("for +31612345678"), "the number is shown as Telegram has it");
        assert!(app_prompt(true).problem.contains("did not accept") && app_prompt(false).problem.is_empty());
        assert!(password_prompt("pet").detail.ends_with("Hint: pet") && password_prompt("").fields[0].secret);
    }
}
