use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use wait_timeout::ChildExt;

const PROJECTIVY_URL: &str = "https://github.com/spocky/miproja1/releases/download/4.73/ProjectivyLauncher-4.73-c98-xda-release.apk";
const PROJECTIVY_SHA256: &str = "74e33af1e5e13831a69a0669d2b68b02150bf9df1ae1df437dc67e3f8d52c790";
const PROJECTIVY_COMPONENT: &str = "com.spocky.projengmenu/.ui.home.MainActivity";

#[derive(Clone, Debug, PartialEq)]
pub enum Risk {
    Optional,
    Caution,
    Protected,
}

#[derive(Clone, Debug)]
pub struct PackageInfo {
    pub name: String,
    pub label: String,
    pub installed: bool,
    pub running: bool,
    pub third_party: bool,
    pub risk: Risk,
}

#[derive(Clone, Debug, Default)]
pub struct DeviceSnapshot {
    pub serial: String,
    pub brand: String,
    pub model: String,
    pub android: String,
    pub abi: String,
    pub launcher: String,
    pub packages: Vec<PackageInfo>,
}

pub struct Adb {
    path: PathBuf,
}

impl Adb {
    pub fn new() -> Result<Self> {
        if let Some(path) = locate_adb() {
            return Ok(Self { path });
        }
        let path = install_adb()?;
        Ok(Self { path })
    }

    pub fn connect(&self, ip: Ipv4Addr, port: u16) -> Result<DeviceSnapshot> {
        let serial = format!("{ip}:{port}");
        self.run(&["connect", &serial], Duration::from_secs(40))?;
        let devices = self.run(&["devices"], Duration::from_secs(20))?;
        let state = devices.lines().find_map(|line| {
            let mut columns = line.split_whitespace();
            (columns.next() == Some(serial.as_str())).then(|| columns.next().unwrap_or("unknown"))
        });
        match state {
            Some("device") => self.inspect(&serial),
            Some("unauthorized") => {
                bail!("Подтвердите отладку на экране телевизора и повторите подключение")
            }
            Some("offline") => {
                bail!("Телевизор отвечает, но ADB offline. Переподключите ADB на телевизоре")
            }
            _ => {
                bail!("Не удалось подключиться к {serial}. Проверьте IP, порт и ADB на телевизоре")
            }
        }
    }

    pub fn inspect(&self, serial: &str) -> Result<DeviceSnapshot> {
        let brand = self
            .shell(serial, &["getprop", "ro.product.brand"])?
            .trim()
            .to_string();
        let model = self
            .shell(serial, &["getprop", "ro.product.model"])?
            .trim()
            .to_string();
        let android = self
            .shell(serial, &["getprop", "ro.build.version.release"])?
            .trim()
            .to_string();
        let abi = self
            .shell(serial, &["getprop", "ro.product.cpu.abi"])?
            .trim()
            .to_string();
        let launcher = self
            .run_for_device(
                serial,
                &["shell", "cmd", "shortcut", "get-default-launcher"],
                Duration::from_secs(20),
            )
            .unwrap_or_default()
            .lines()
            .find_map(|line| line.strip_prefix("Launcher: "))
            .unwrap_or("не определён")
            .to_string();

        let all =
            package_names(&self.shell(serial, &["pm", "list", "packages", "-u", "--user", "0"])?);
        let installed =
            package_names(&self.shell(serial, &["pm", "list", "packages", "--user", "0"])?);
        let third_party =
            package_names(&self.shell(serial, &["pm", "list", "packages", "-3", "--user", "0"])?);
        let processes = self.shell(serial, &["ps", "-A"])?;
        let running = running_packages(&processes);
        let launcher_package = launcher
            .split('/')
            .next()
            .and_then(|value| value.rsplit('{').next())
            .unwrap_or_default();

        let mut packages: Vec<_> = all
            .into_iter()
            .map(|name| {
                let is_installed = installed.contains(&name);
                let risk = package_risk(&name, third_party.contains(&name), launcher_package);
                PackageInfo {
                    label: package_label(&name).to_string(),
                    installed: is_installed,
                    running: running.contains(&name),
                    third_party: third_party.contains(&name),
                    risk,
                    name,
                }
            })
            .collect();
        packages.sort_by(|a, b| {
            a.label
                .to_lowercase()
                .cmp(&b.label.to_lowercase())
                .then(a.name.cmp(&b.name))
        });

        Ok(DeviceSnapshot {
            serial: serial.to_string(),
            brand,
            model,
            android,
            abi,
            launcher,
            packages,
        })
    }

    pub fn disable_packages(&self, serial: &str, packages: &[String]) -> Result<DeviceSnapshot> {
        if packages.is_empty() {
            bail!("Не выбраны пакеты для отключения");
        }
        let before = self.inspect(serial)?;
        let launcher_package = before
            .launcher
            .split('/')
            .next()
            .and_then(|value| value.rsplit('{').next())
            .unwrap_or_default();
        let mut errors = Vec::new();
        for name in packages {
            validate_package_name(name)?;
            let Some(package) = before.packages.iter().find(|package| &package.name == name) else {
                errors.push(format!("{name}: пакет не найден"));
                continue;
            };
            if package.risk == Risk::Protected || launcher_package == *name {
                errors.push(format!("{name}: отключение заблокировано для безопасности"));
                continue;
            }
            let disabled = self
                .shell(serial, &["pm", "uninstall", "--user", "0", name])
                .or_else(|_| self.shell(serial, &["pm", "disable-user", "--user", "0", name]));
            if let Err(error) = disabled {
                errors.push(format!("{name}: {error}"));
            }
        }
        let snapshot = self.inspect(serial)?;
        for name in packages {
            if snapshot
                .packages
                .iter()
                .find(|package| &package.name == name)
                .is_some_and(|package| package.installed)
            {
                errors.push(format!("{name}: пакет всё ещё установлен"));
            }
        }
        if !errors.is_empty() {
            bail!("Часть операций не выполнена:\n{}", errors.join("\n"));
        }
        Ok(snapshot)
    }

    pub fn restore_packages(&self, serial: &str, packages: &[String]) -> Result<DeviceSnapshot> {
        if packages.is_empty() {
            bail!("Не выбраны пакеты для восстановления");
        }
        let mut errors = Vec::new();
        for name in packages {
            validate_package_name(name)?;
            let restored = self
                .shell(serial, &["cmd", "package", "install-existing", name])
                .or_else(|_| self.shell(serial, &["pm", "enable", name]));
            if let Err(error) = restored {
                errors.push(format!("{name}: {error}"));
            }
        }
        let snapshot = self.inspect(serial)?;
        for name in packages {
            if snapshot
                .packages
                .iter()
                .find(|package| &package.name == name)
                .is_none_or(|package| !package.installed)
            {
                errors.push(format!("{name}: пакет не удалось восстановить"));
            }
        }
        if !errors.is_empty() {
            bail!("Часть операций не выполнена:\n{}", errors.join("\n"));
        }
        Ok(snapshot)
    }

    pub fn install_projectivy(&self, serial: &str) -> Result<String> {
        let directory = app_directory()?.join("downloads");
        fs::create_dir_all(&directory)?;
        let apk = directory.join("ProjectivyLauncher-4.73.apk");
        download(PROJECTIVY_URL, &apk, PROJECTIVY_SHA256)?;
        self.run_for_device(
            serial,
            &["install", "-r", path_text(&apk)?],
            Duration::from_secs(120),
        )?;
        self.shell(
            serial,
            &["cmd", "package", "set-home-activity", PROJECTIVY_COMPONENT],
        )?;
        let launcher = self.shell(serial, &["cmd", "shortcut", "get-default-launcher"])?;
        if !launcher.contains("com.spocky.projengmenu") {
            bail!("Android не подтвердил Projectivy как главный лаунчер: {launcher}");
        }
        self.shell(
            serial,
            &[
                "am",
                "start",
                "-a",
                "android.intent.action.MAIN",
                "-c",
                "android.intent.category.HOME",
            ],
        )?;
        Ok(
            "Projectivy 4.73 установлен и назначен главным экраном. Старый лаунчер не отключён."
                .to_string(),
        )
    }

    pub fn install_apk(&self, serial: &str, apk: &Path, package: &str) -> Result<DeviceSnapshot> {
        validate_package_name(package)?;
        self.run_for_device(
            serial,
            &["install", "-r", path_text(apk)?],
            Duration::from_secs(180),
        )?;
        let snapshot = self.inspect(serial)?;
        if snapshot
            .packages
            .iter()
            .find(|item| item.name == package)
            .is_none_or(|item| !item.installed)
        {
            bail!("Android не подтвердил установку пакета {package}");
        }
        Ok(snapshot)
    }

    fn shell(&self, serial: &str, args: &[&str]) -> Result<String> {
        let mut command = vec!["shell"];
        command.extend_from_slice(args);
        let output = self.run_for_device(serial, &command, Duration::from_secs(35))?;
        if output.contains("Failure") || output.contains("Exception") || output.starts_with("Error")
        {
            bail!(output);
        }
        Ok(output)
    }

    fn run_for_device(&self, serial: &str, args: &[&str], timeout: Duration) -> Result<String> {
        let mut command = vec!["-s", serial];
        command.extend_from_slice(args);
        self.run(&command, timeout)
    }

    fn run(&self, args: &[&str], timeout: Duration) -> Result<String> {
        let mut child = Command::new(&self.path)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("Не удалось запустить {}", self.path.display()))?;
        let status = child.wait_timeout(timeout)?;
        if status.is_none() {
            child.kill()?;
            let _ = child.wait_with_output();
            bail!("ADB не ответил за {} секунд", timeout.as_secs());
        }
        let output = child.wait_with_output()?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if output.status.success() {
            Ok(stdout)
        } else {
            let message = if stderr.is_empty() { stdout } else { stderr };
            bail!(if message.is_empty() {
                "ADB завершился с ошибкой".to_string()
            } else {
                message
            })
        }
    }
}

pub fn discover_adb_devices() -> Result<Vec<Ipv4Addr>> {
    let mut prefixes = HashSet::new();
    for interface in if_addrs::get_if_addrs()? {
        if let std::net::IpAddr::V4(ip) = interface.ip() {
            if ip.is_private() && !ip.is_loopback() {
                let octets = ip.octets();
                prefixes.insert([octets[0], octets[1], octets[2]]);
            }
        }
    }
    if prefixes.is_empty() {
        bail!("Не найдена частная IPv4-сеть");
    }

    let found = Arc::new(Mutex::new(Vec::new()));
    for prefix in prefixes {
        thread::scope(|scope| {
            for worker in 0..32 {
                let found = Arc::clone(&found);
                scope.spawn(move || {
                    let mut index = worker + 1;
                    while index < 255 {
                        let ip = Ipv4Addr::new(prefix[0], prefix[1], prefix[2], index as u8);
                        let address = SocketAddr::new(ip.into(), 5555);
                        if TcpStream::connect_timeout(&address, Duration::from_millis(140)).is_ok()
                        {
                            found.lock().unwrap().push(ip);
                        }
                        index += 32;
                    }
                });
            }
        });
    }
    let mut devices = Arc::try_unwrap(found)
        .map_err(|_| anyhow::anyhow!("Сканер ещё выполняется"))?
        .into_inner()?;
    devices.sort();
    devices.dedup();
    Ok(devices)
}

fn locate_adb() -> Option<PathBuf> {
    let executable = if cfg!(windows) { "adb.exe" } else { "adb" };
    if let Some(path) = env::var_os("TV_ASSISTANT_ADB")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
    {
        return Some(path);
    }
    if let Ok(current_executable) = env::current_exe() {
        if let Some(bin) = current_executable.parent() {
            for directory in [
                bin.join("platform-tools"),
                bin.join("Resources").join("platform-tools"),
                bin.parent()
                    .map(|contents| contents.join("Resources").join("platform-tools"))
                    .unwrap_or_default(),
            ] {
                let candidate = directory.join(executable);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    if let Some(paths) = env::var_os("PATH") {
        for directory in env::split_paths(&paths) {
            let candidate = directory.join(executable);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let managed = app_directory()
        .ok()?
        .join("platform-tools")
        .join(executable);
    if managed.is_file() {
        return Some(managed);
    }
    #[cfg(target_os = "macos")]
    {
        let home = env::var_os("HOME").map(PathBuf::from)?;
        for candidate in [
            PathBuf::from("/opt/homebrew/bin/adb"),
            PathBuf::from("/usr/local/bin/adb"),
            home.join("Library/Android/sdk/platform-tools/adb"),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn install_adb() -> Result<PathBuf> {
    let url = if cfg!(target_os = "macos") {
        "https://dl.google.com/android/repository/platform-tools-latest-darwin.zip"
    } else if cfg!(target_os = "linux") {
        "https://dl.google.com/android/repository/platform-tools-latest-linux.zip"
    } else if cfg!(windows) {
        "https://dl.google.com/android/repository/platform-tools-latest-windows.zip"
    } else {
        bail!("Эта операционная система не поддерживается");
    };
    let directory = app_directory()?;
    fs::create_dir_all(&directory)?;
    let archive = directory.join("platform-tools.zip");
    download(url, &archive, None)?;
    let file = fs::File::open(&archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        if !relative.starts_with(Path::new("platform-tools")) {
            continue;
        }
        let destination = directory.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&destination)?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = fs::File::create(&destination)?;
        std::io::copy(&mut entry, &mut output)?;
    }
    fs::remove_file(&archive)?;
    let executable =
        directory
            .join("platform-tools")
            .join(if cfg!(windows) { "adb.exe" } else { "adb" });
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&executable)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions)?;
    }
    Ok(executable)
}

pub(crate) fn download<'a>(
    url: &str,
    destination: &Path,
    expected_sha256: impl Into<Option<&'a str>>,
) -> Result<()> {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(180))
        .user_agent("TV-Assistant/0.1")
        .build()?
        .get(url)
        .send()?
        .error_for_status()?;
    let bytes = response.bytes()?;
    if let Some(expected) = expected_sha256.into() {
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if actual != expected {
            bail!("SHA-256 не совпал для {url}");
        }
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(destination, &bytes)?;
    Ok(())
}

pub(crate) fn app_directory() -> Result<PathBuf> {
    let project = ProjectDirs::from("ru", "TV Assistant", "TV Assistant")
        .context("Не найдена папка данных приложения")?;
    Ok(project.data_dir().to_path_buf())
}

pub(crate) fn path_text(path: &Path) -> Result<&str> {
    path.to_str().context("Некорректный путь к файлу")
}

fn validate_package_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_'))
    {
        bail!("Некорректное имя пакета");
    }
    Ok(())
}

fn package_names(output: &str) -> HashSet<String> {
    output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("package:").map(str::to_string))
        .collect()
}

fn running_packages(output: &str) -> HashSet<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().last())
        .filter(|name| name.contains('.'))
        .map(|name| name.split(':').next().unwrap_or(name).to_string())
        .collect()
}

fn package_risk(name: &str, third_party: bool, launcher: &str) -> Risk {
    const OPTIONAL: &[&str] = &[
        "com.vk.tv",
        "ru.rutube.app",
        "ru.kinopoisk.yandex.tv",
        "com.yandex.tv.music",
        "com.yandex.tv.live",
        "com.yandex.tv.input.efir",
        "com.yandex.tv.daydream",
        "com.yandex.browser.tv",
        "com.yandex.android.advid",
        "com.yandex.tv.bugreportsender",
        "com.yandex.launcher.updaterapp",
        "com.yandex.devices.appstore",
    ];
    const CAUTION: &[&str] = &[
        "com.yandex.tv.alice",
        "com.yandex.io.sdk",
        "com.yandex.tv.home",
    ];
    const PROTECTED_PREFIXES: &[&str] = &[
        "android",
        "com.android.",
        "com.mediatek.",
        "com.mtk.",
        "com.ctv.",
        "com.cultraview.",
        "com.seraphic.",
        "com.yandex.tv.services",
        "com.yandex.server.",
        "com.yandex.devices.player",
        "com.yandex.devices.farm.",
        "com.yandex.tv.settings",
        "com.yandex.tv.setupwizard",
        "com.yandex.tv.vendor.",
        "com.yandex.webview",
        "com.yandex.tv.webplayer",
        "com.yandex.tv.videoplayer",
        "com.yandex.tv.ytplayer",
        "ru.yandex.androidkeyboard.tv",
        "com.spocky.projengmenu",
        "su.happ.proxyutility",
    ];
    if OPTIONAL.contains(&name) {
        Risk::Optional
    } else if CAUTION.contains(&name) {
        Risk::Caution
    } else if name == launcher
        || PROTECTED_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        || name.contains("factorymenu")
        || name.contains("autotest")
    {
        Risk::Protected
    } else if third_party {
        Risk::Optional
    } else {
        Risk::Caution
    }
}

fn package_label(name: &str) -> &str {
    match name {
        "com.vk.tv" => "VK Видео",
        "ru.rutube.app" => "RuTube",
        "ru.kinopoisk.yandex.tv" => "Кинопоиск",
        "com.yandex.tv.music" => "Яндекс Музыка",
        "com.yandex.tv.alice" => "Алиса",
        "com.yandex.io.sdk" => "Яндекс IO / голосовая связь",
        "com.yandex.tv.live" => "Каналы Яндекс ТВ",
        "com.yandex.tv.input.efir" => "Эфирные каналы Яндекса",
        "com.yandex.tv.daydream" => "Заставка Яндекса",
        "com.yandex.browser.tv" => "Браузер Яндекса",
        "com.yandex.android.advid" => "Рекламный идентификатор",
        "com.yandex.tv.bugreportsender" => "Отчёты об ошибках Яндекса",
        "com.yandex.launcher.updaterapp" => "Обновлятор Яндекса",
        "com.yandex.devices.appstore" => "Магазин приложений Яндекса",
        "com.yandex.tv.home" => "Лаунчер Яндекса",
        "com.spocky.projengmenu" => "Projectivy Launcher",
        "su.happ.proxyutility" => "Happ",
        _ => name,
    }
}
