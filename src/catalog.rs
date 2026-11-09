use crate::adb::{Adb, DeviceSnapshot, app_directory, download};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct CatalogItem {
    pub id: String,
    pub name: String,
    pub package: String,
    pub description: String,
    pub category: String,
    pub icon: String,
    pub version: String,
    pub asset_name: String,
    pub download_url: String,
    pub sha256: Option<String>,
    pub size: u64,
    pub installed: bool,
}

#[derive(Clone, Copy)]
struct Definition {
    id: &'static str,
    name: &'static str,
    package: &'static str,
    description: &'static str,
    category: &'static str,
    icon: &'static str,
    repository: &'static str,
    asset_prefix: &'static str,
    asset_suffix: &'static str,
    abi_specific: bool,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
    size: u64,
}

const CATALOG: &[Definition] = &[
    Definition {
        id: "projectivy",
        name: "Projectivy Launcher",
        package: "com.spocky.projengmenu",
        description: "Лёгкий домашний экран с категориями, HDMI и настройкой внешнего вида.",
        category: "Лаунчеры",
        icon: "home",
        repository: "spocky/miproja1",
        asset_prefix: "ProjectivyLauncher-",
        asset_suffix: "-xda-release.apk",
        abi_specific: false,
    },
    Definition {
        id: "flauncher",
        name: "FLauncher",
        package: "me.efesser.flauncher",
        description: "Минималистичный лаунчер с открытым кодом, без рекламы и рекомендаций.",
        category: "Лаунчеры",
        icon: "grid",
        repository: "CocoCR300/flauncher",
        asset_prefix: "flauncher-",
        asset_suffix: "-release.apk",
        abi_specific: true,
    },
    Definition {
        id: "smarttube",
        name: "SmartTube",
        package: "com.teamsmart.videomanager",
        description: "Удобный клиент YouTube для ТВ с пультом и без рекламы.",
        category: "Видео",
        icon: "tv",
        repository: "yuliskov/SmartTube",
        asset_prefix: "SmartTube_stable_",
        asset_suffix: ".apk",
        abi_specific: true,
    },
    Definition {
        id: "tvbro",
        name: "TV Bro",
        package: "com.phlox.tvwebbrowser",
        description: "Браузер, адаптированный для Android TV и управления пультом.",
        category: "Интернет",
        icon: "external",
        repository: "truefedex/tv-bro",
        asset_prefix: "tvbro-",
        asset_suffix: ".apk",
        abi_specific: true,
    },
    Definition {
        id: "material-files",
        name: "Material Files",
        package: "me.zhanghai.android.files",
        description: "Файловый менеджер с открытым кодом для локальных и сетевых файлов.",
        category: "Утилиты",
        icon: "package",
        repository: "zhanghai/MaterialFiles",
        asset_prefix: "app-release-universal.apk",
        asset_suffix: "app-release-universal.apk",
        abi_specific: false,
    },
    Definition {
        id: "happ",
        name: "Happ",
        package: "su.happ.proxyutility",
        description: "Клиент для личных прокси-серверов на базе Xray с импортом по QR-коду.",
        category: "Сеть",
        icon: "wifi",
        repository: "Happ-proxy/happ-android",
        asset_prefix: "Happ.apk",
        asset_suffix: "Happ.apk",
        abi_specific: false,
    },
];

pub fn fetch_catalog(device: &DeviceSnapshot) -> Result<Vec<CatalogItem>> {
    let mut items = Vec::new();
    for definition in CATALOG {
        let url = format!(
            "https://api.github.com/repos/{}/releases/latest",
            definition.repository
        );
        let release: GitHubRelease = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("TV-Assistant/0.1")
            .build()?
            .get(url)
            .send()?
            .error_for_status()?
            .json()
            .with_context(|| format!("Не удалось получить релиз {}", definition.name))?;
        let asset = release
            .assets
            .iter()
            .find(|asset| asset_matches(definition, asset, &device.abi))
            .with_context(|| format!("Не найден APK {} для {}", definition.name, device.abi))?;
        items.push(CatalogItem {
            id: definition.id.to_string(),
            name: definition.name.to_string(),
            package: definition.package.to_string(),
            description: definition.description.to_string(),
            category: definition.category.to_string(),
            icon: definition.icon.to_string(),
            version: release.tag_name,
            asset_name: asset.name.clone(),
            download_url: asset.browser_download_url.clone(),
            sha256: asset
                .digest
                .as_deref()
                .and_then(|digest| digest.strip_prefix("sha256:"))
                .map(str::to_string),
            size: asset.size,
            installed: device
                .packages
                .iter()
                .any(|package| package.name == definition.package && package.installed),
        });
    }
    Ok(items)
}

pub fn install_catalog_app(
    device: &DeviceSnapshot,
    item: &CatalogItem,
) -> Result<(String, DeviceSnapshot)> {
    ensure_compatible(item, &device.android)?;
    let directory = app_directory()?.join("downloads");
    let apk = directory.join(&item.asset_name);
    download(&item.download_url, &apk, item.sha256.as_deref())?;
    let adb = Adb::new()?;
    let snapshot = adb.install_apk(&device.serial, &apk, &item.package)?;
    Ok((
        format!("{} {} установлен на телевизор", item.name, item.version),
        snapshot,
    ))
}

fn asset_matches(definition: &Definition, asset: &GitHubAsset, abi: &str) -> bool {
    if !asset.name.starts_with(definition.asset_prefix)
        || !asset.name.ends_with(definition.asset_suffix)
    {
        return false;
    }
    if definition.abi_specific && matches!(abi, "arm64-v8a" | "armeabi-v7a") {
        asset.name.contains(abi)
    } else if definition.abi_specific {
        asset.name.contains("universal") || asset.name.contains("geckoExcluded")
    } else {
        true
    }
}

pub fn format_size(size: u64) -> String {
    if size >= 1024 * 1024 {
        format!("{:.1} МБ", size as f64 / (1024. * 1024.))
    } else {
        format!("{:.0} КБ", size as f64 / 1024.)
    }
}

pub fn ensure_compatible(item: &CatalogItem, android: &str) -> Result<()> {
    let major = android
        .split('.')
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0);
    if item.id == "happ" && major < 7 {
        bail!("Happ требует Android 7 или новее");
    }
    if item.id == "projectivy" && major < 6 {
        bail!("Projectivy требует Android 6 или новее");
    }
    Ok(())
}
