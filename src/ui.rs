use crate::adb::{Adb, DeviceSnapshot, PackageInfo, Risk, discover_adb_devices};
use crate::assets::{Assets, resource_path};
use crate::catalog::{CatalogItem, fetch_catalog, format_size, install_catalog_app};
use anyhow::Result;
use gpui::{
    AnyElement, App, Application, Bounds, Context, ElementId, FocusHandle, KeyBinding,
    KeyDownEvent, MouseButton, Rgba, SharedString, Stateful, Svg, TitlebarOptions, Window,
    WindowBounds, WindowOptions, actions, div, prelude::*, px, rgb, size, svg,
};
use std::collections::HashSet;
use std::net::Ipv4Addr;

const fn color(hex: u32) -> Rgba {
    let [_, r, g, b] = hex.to_be_bytes();
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

const BACKGROUND: Rgba = color(0x11151c);
const PANEL: Rgba = color(0x1a202b);
const PANEL_ALT: Rgba = color(0x202837);
const BORDER: Rgba = color(0x303b4d);
const TEXT: Rgba = color(0xe8edf5);
const MUTED: Rgba = color(0x94a3b8);
const ACCENT: Rgba = color(0x5677f5);
const GREEN: Rgba = color(0x268a62);
const RED: Rgba = color(0xb94b5b);
const ORANGE: Rgba = color(0xb7792d);
const SIDEBAR: Rgba = color(0x151b26);
const ACCENT_SOFT: Rgba = color(0x202f57);
const GREEN_SOFT: Rgba = color(0x183b34);
const RED_SOFT: Rgba = color(0x42242d);
const ORANGE_SOFT: Rgba = color(0x40341f);

actions!(tv_assistant, [Quit]);

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Connection,
    Packages,
    Catalog,
    Launcher,
    Guide,
}

pub struct TvAssistant {
    page: Page,
    focus_handle: FocusHandle,
    ip_input: String,
    editing_ip: bool,
    ip_selected: bool,
    device: Option<DeviceSnapshot>,
    discovered: Vec<Ipv4Addr>,
    selected: HashSet<String>,
    show_all: bool,
    busy: bool,
    status: String,
    logs: Vec<String>,
    confirm_disable: bool,
    catalog: Vec<CatalogItem>,
    catalog_loaded: bool,
}

impl TvAssistant {
    fn new(cx: &mut Context<Self>, initial_device: Option<DeviceSnapshot>) -> Self {
        let (page, ip_input, status) = match initial_device.as_ref() {
            Some(device) => (
                Page::Packages,
                device
                    .serial
                    .split(':')
                    .next()
                    .unwrap_or_default()
                    .to_string(),
                format!(
                    "Подключено: {} {} · Android {}",
                    device.brand, device.model, device.android
                ),
            ),
            None => (
                Page::Connection,
                String::new(),
                "Включите ADB на телевизоре, затем подключитесь по Wi-Fi".to_string(),
            ),
        };
        Self {
            page,
            focus_handle: cx.focus_handle(),
            ip_input,
            editing_ip: false,
            ip_selected: false,
            device: initial_device,
            discovered: Vec::new(),
            selected: HashSet::new(),
            show_all: false,
            busy: false,
            status,
            logs: vec![
                "Приложение готово. Изменения выполняются только по вашей команде.".to_string(),
            ],
            confirm_disable: false,
            catalog: Vec::new(),
            catalog_loaded: false,
        }
    }

    fn log(&mut self, message: impl Into<String>) {
        self.logs.push(message.into());
        if self.logs.len() > 40 {
            self.logs.remove(0);
        }
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let ip = match self.ip_input.parse::<Ipv4Addr>() {
            Ok(ip) => ip,
            Err(_) => {
                self.status = "Введите корректный IPv4-адрес, например 192.168.0.100".to_string();
                cx.notify();
                return;
            }
        };
        self.busy = true;
        self.status = format!("Подключение к {ip}:5555…");
        self.log(format!("Поиск ADB и подключение к {ip}:5555"));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let task = cx
                .background_executor()
                .spawn(async move { Adb::new()?.connect(ip, 5555) });
            let result = task.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(device) => {
                        this.status = format!(
                            "Подключено: {} {} · Android {}",
                            device.brand, device.model, device.android
                        );
                        this.log(format!(
                            "Подключено {} · пакетов: {}",
                            device.serial,
                            device.packages.len()
                        ));
                        this.device = Some(device);
                        this.page = Page::Packages;
                    }
                    Err(error) => {
                        this.status = error.to_string();
                        this.log(format!("Ошибка подключения: {error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn discover(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "Сканирование локальной сети по ADB-порту 5555…".to_string();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let task = cx
                .background_executor()
                .spawn(async move { discover_adb_devices() });
            let result = task.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(devices) => {
                        this.status = if devices.is_empty() {
                            "ADB-устройства не найдены. Проверьте ADB на ТВ или введите IP вручную"
                                .to_string()
                        } else {
                            format!("Найдено ADB-устройств: {}", devices.len())
                        };
                        if let Some(first) = devices.first() {
                            this.ip_input = first.to_string();
                        }
                        this.log(format!(
                            "Найдено: {}",
                            devices
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                        this.discovered = devices;
                    }
                    Err(error) => {
                        this.status = format!("Ошибка сканирования: {error}");
                        this.log(format!("Ошибка сканирования: {error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(serial) = self.device.as_ref().map(|device| device.serial.clone()) else {
            return;
        };
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "Чтение состояния телевизора…".to_string();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let task = cx
                .background_executor()
                .spawn(async move { Adb::new()?.inspect(&serial) });
            let result = task.await;
            this.update(cx, |this, cx| {
                this.finish_snapshot(result, "Список пакетов обновлён", cx)
            })
            .ok();
        })
        .detach();
    }

    fn load_catalog(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(device) = self.device.clone() else {
            self.status = "Сначала подключите телевизор".to_string();
            self.page = Page::Connection;
            cx.notify();
            return;
        };
        self.busy = true;
        self.status = "Загрузка официальных релизов…".to_string();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let task = cx
                .background_executor()
                .spawn(async move { fetch_catalog(&device) });
            let result = task.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(items) => {
                        this.status = format!("В каталоге приложений: {}", items.len());
                        this.log(format!("Каталог обновлён: {} приложений", items.len()));
                        this.catalog = items;
                        this.catalog_loaded = true;
                    }
                    Err(error) => {
                        this.catalog_loaded = false;
                        this.status = format!("Не удалось загрузить каталог: {error}");
                        this.log(format!("Ошибка каталога: {error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn install_catalog_item(&mut self, id: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(device) = self.device.clone() else {
            self.status = "Сначала подключите телевизор".to_string();
            cx.notify();
            return;
        };
        let Some(item) = self.catalog.iter().find(|item| item.id == id).cloned() else {
            self.status = "Сначала обновите каталог".to_string();
            cx.notify();
            return;
        };
        self.busy = true;
        self.status = format!("Установка {}…", item.name);
        self.log(format!("Загрузка {} из официального релиза", item.name));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let task = cx
                .background_executor()
                .spawn(async move { install_catalog_app(&device, &item) });
            let result = task.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((message, snapshot)) => {
                        this.status = message.clone();
                        this.log(message);
                        this.device = Some(snapshot);
                        if let Some(item) = this.catalog.iter_mut().find(|item| item.id == id) {
                            item.installed = true;
                        }
                    }
                    Err(error) => {
                        this.status = error.to_string();
                        this.log(format!("Ошибка установки: {error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn change_packages(&mut self, restore: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(device) = self.device.as_ref() else {
            return;
        };
        let serial = device.serial.clone();
        let packages: Vec<String> = device
            .packages
            .iter()
            .filter(|package| self.selected.contains(&package.name) && package.installed != restore)
            .map(|package| package.name.clone())
            .collect();
        if packages.is_empty() {
            self.status = if restore {
                "Выберите отключённые пакеты для восстановления".to_string()
            } else {
                "Выберите установленные пакеты для отключения".to_string()
            };
            cx.notify();
            return;
        }
        if !restore && !self.confirm_disable {
            self.confirm_disable = true;
            self.status = format!("Подтвердите отключение пакетов: {}", packages.len());
            cx.notify();
            return;
        }
        self.confirm_disable = false;
        self.busy = true;
        self.status = if restore {
            "Восстановление пакетов…"
        } else {
            "Отключение пакетов…"
        }
        .to_string();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let task = cx.background_executor().spawn(async move {
                let adb = Adb::new()?;
                if restore {
                    adb.restore_packages(&serial, &packages)
                } else {
                    adb.disable_packages(&serial, &packages)
                }
            });
            let result = task.await;
            this.update(cx, |this, cx| {
                let message = if restore {
                    "Выбранные пакеты восстановлены"
                } else {
                    "Выбранные пакеты отключены"
                };
                this.finish_snapshot(result, message, cx)
            })
            .ok();
        })
        .detach();
    }

    fn finish_snapshot(
        &mut self,
        result: Result<DeviceSnapshot>,
        message: &str,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        match result {
            Ok(device) => {
                self.status = message.to_string();
                self.log(message);
                self.device = Some(device);
                self.selected.clear();
            }
            Err(error) => {
                self.status = error.to_string();
                self.log(format!("Ошибка: {error:#}"));
            }
        }
        cx.notify();
    }

    fn install_projectivy(&mut self, cx: &mut Context<Self>) {
        let Some(serial) = self.device.as_ref().map(|device| device.serial.clone()) else {
            self.status = "Сначала подключите телевизор".to_string();
            self.page = Page::Connection;
            cx.notify();
            return;
        };
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "Загрузка и установка Projectivy 4.73…".to_string();
        self.log("Загрузка Projectivy из официального релиза spocky/miproja1");
        cx.notify();
        cx.spawn(async move |this, cx| {
            let task = cx.background_executor().spawn(async move {
                let adb = Adb::new()?;
                let message = adb.install_projectivy(&serial)?;
                let snapshot = adb.inspect(&serial)?;
                Ok((message, snapshot))
            });
            let result: Result<(String, DeviceSnapshot)> = task.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((message, device)) => {
                        this.status = message.clone();
                        this.log(message);
                        this.device = Some(device);
                    }
                    Err(error) => {
                        this.status = error.to_string();
                        this.log(format!("Ошибка установки Projectivy: {error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn select_recommended(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if let Some(device) = self.device.as_ref() {
            self.selected = device
                .packages
                .iter()
                .filter(|package| package.installed && package.risk == Risk::Optional)
                .map(|package| package.name.clone())
                .collect();
            self.status = format!("Выбрано необязательных пакетов: {}", self.selected.len());
            cx.notify();
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.editing_ip {
            return;
        }
        let stroke = &event.keystroke;
        match stroke.key.as_str() {
            "escape" | "enter" => self.editing_ip = false,
            "backspace" => {
                if self.ip_selected {
                    self.ip_input.clear();
                } else {
                    self.ip_input.pop();
                }
                self.ip_selected = false;
            }
            "v" if stroke.modifiers.platform || stroke.modifiers.control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.ip_input = text
                        .chars()
                        .filter(|character| character.is_ascii_digit() || *character == '.')
                        .take(15)
                        .collect();
                    self.ip_selected = false;
                }
            }
            _ => {
                if let Some(text) = stroke.key_char.as_ref() {
                    if self.ip_selected {
                        self.ip_input.clear();
                        self.ip_selected = false;
                    }
                    for character in text
                        .chars()
                        .filter(|character| character.is_ascii_digit() || *character == '.')
                    {
                        if self.ip_input.len() < 15 {
                            self.ip_input.push(character);
                        }
                    }
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn navigation(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut sidebar = div()
            .w(px(250.))
            .h_full()
            .bg(SIDEBAR)
            .border_r_1()
            .border_color(BORDER)
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .pb_4()
                    .mb_2()
                    .border_b_1()
                    .border_color(BORDER)
                    .child(
                        div()
                            .size_10()
                            .rounded_lg()
                            .bg(ACCENT_SOFT)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon("tv", ACCENT, px(27.))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("TV Assistant"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(MUTED)
                                    .child("ANDROID TV TOOLKIT"),
                            ),
                    ),
            );
        for (page, title, icon_name) in [
            (Page::Connection, "Подключение", "wifi"),
            (Page::Packages, "Пакеты и сервисы", "grid"),
            (Page::Catalog, "Каталог приложений", "download"),
            (Page::Launcher, "Лаунчер", "home"),
            (Page::Guide, "Включение ADB", "terminal"),
        ] {
            let active = self.page == page;
            sidebar = sidebar.child(
                icon_action_button(
                    SharedString::from(format!("nav-{title}")),
                    icon_name,
                    title,
                    if active { ACCENT } else { SIDEBAR },
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.page = page;
                    this.confirm_disable = false;
                    if page == Page::Catalog && !this.catalog_loaded {
                        this.load_catalog(cx);
                    }
                    cx.notify();
                })),
            );
        }
        sidebar = sidebar.child(
            div()
                .flex_1()
                .child(div().text_xs().text_color(MUTED).pt_4().child(
                    "Android TV и приставки с включённым ADB. Tizen и webOS не поддерживаются.",
                )),
        );
        sidebar.child(
            card()
                .bg(PANEL)
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_xs().text_color(MUTED).child("РАЗРАБОТЧИК"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .size_8()
                                .rounded_full()
                                .bg(ACCENT_SOFT)
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_xs()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(ACCENT)
                                .child("IF"),
                        )
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("iafilin"),
                        )
                        .child(
                            div()
                                .id("developer-vk")
                                .cursor_pointer()
                                .text_color(ACCENT)
                                .child(icon("external", ACCENT, px(16.)))
                                .on_click(
                                    cx.listener(|_, _, _, cx| cx.open_url("https://vk.ru/iafilin")),
                                ),
                        ),
                )
                .child(
                    div()
                        .id("developer-email")
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(MUTED)
                        .child(icon("mail", MUTED, px(15.)))
                        .child("iafilin@vk.ru")
                        .on_click(cx.listener(|_, _, _, cx| cx.open_url("mailto:iafilin@vk.ru"))),
                ),
        )
    }

    fn connection_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let ip_display = if self.ip_input.is_empty() {
            "192.168.0.100".to_string()
        } else {
            self.ip_input.clone()
        };
        let mut page = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(icon("wifi", ACCENT, px(28.)))
                    .child(title("Подключение к телевизору")),
            )
            .child(subtitle("Пользователь включает ADB на ТВ самостоятельно. Программа выполняет остальные действия по Wi-Fi."))
            .child(
                card()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(label("IP-адрес телевизора"))
                    .child(
                        div()
                            .id("ip-field")
                            .w(px(330.))
                            .p_3()
                            .rounded_md()
                            .border_1()
                            .border_color(if self.editing_ip { ACCENT } else { BORDER })
                            .bg(BACKGROUND)
                            .cursor(gpui::CursorStyle::IBeam)
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                this.editing_ip = true;
                                this.ip_selected = true;
                                cx.notify();
                            }))
                            .child(div().text_color(if self.ip_input.is_empty() { MUTED } else { TEXT }).child(format!("{ip_display}{}", if self.editing_ip { " |" } else { "" }))),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                tool_button("connect", "wifi", "Подключиться", ACCENT).on_click(cx.listener(|this, _, _, cx| {
                                    this.editing_ip = false;
                                    this.connect(cx);
                                })),
                            )
                            .child(
                                tool_button("discover", "scan", "Найти ADB в сети", PANEL_ALT).on_click(cx.listener(|this, _, _, cx| this.discover(cx))),
                            ),
                    )
                    .child(div().text_xs().text_color(MUTED).child("Порт: 5555 · нажмите на поле, введите IP, Enter завершает ввод")),
            );

        if !self.discovered.is_empty() {
            let mut found = card()
                .flex()
                .flex_col()
                .gap_2()
                .child(label("Найденные устройства"));
            for ip in self.discovered.clone() {
                found = found.child(
                    action_button(
                        SharedString::from(format!("found-{ip}")),
                        ip.to_string(),
                        PANEL_ALT,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ip_input = ip.to_string();
                        this.status = format!("Выбран {ip}. Нажмите «Подключиться»");
                        cx.notify();
                    })),
                );
            }
            page = page.child(found);
        }

        if let Some(device) = self.device.as_ref() {
            page = page.child(
                card()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(label("Текущее устройство"))
                    .child(format!(
                        "{} {} · Android {} · {}\n{}\nЛаунчер: {}",
                        device.brand,
                        device.model,
                        device.android,
                        device.abi,
                        device.serial,
                        device.launcher
                    )),
            );
        }
        page.into_any_element()
    }

    fn packages_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(device) = self.device.as_ref() else {
            return empty_state("Сначала подключите телевизор", cx).into_any_element();
        };
        let indices: Vec<usize> = device
            .packages
            .iter()
            .enumerate()
            .filter(|(_, package)| {
                self.show_all
                    || package.risk == Risk::Optional
                    || !package.installed
                    || self.selected.contains(&package.name)
            })
            .map(|(index, _)| index)
            .collect();
        let installed_selected = device
            .packages
            .iter()
            .filter(|package| self.selected.contains(&package.name) && package.installed)
            .count();
        let removed_selected = device
            .packages
            .iter()
            .filter(|package| self.selected.contains(&package.name) && !package.installed)
            .count();
        let view = cx.entity();
        let mut list = div()
            .id("package-list")
            .w_full()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .border_1()
            .border_color(BORDER)
            .rounded_lg()
            .flex()
            .flex_col();
        for index in indices {
            let package = &device.packages[index];
            list = list.child(package_row(
                index,
                package,
                self.selected.contains(&package.name),
                view.clone(),
            ));
        }

        let running_count = device
            .packages
            .iter()
            .filter(|package| package.running)
            .count();
        let mut page = div()
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(icon("grid", ACCENT, px(28.)))
                    .child(title("Пакеты и фоновые сервисы")),
            )
            .child(subtitle(&format!("{} · отключение только для user 0, без root", device.serial)))
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap_3()
                    .child(stat_card("tv", "Устройство", &format!("{} {}", device.brand, device.model)))
                    .child(stat_card("package", "Всего пакетов", &device.packages.len().to_string()))
                    .child(stat_card("wifi", "Работает", &running_count.to_string()))
                    .child(stat_card("sparkles", "Выбрано", &self.selected.len().to_string())),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(tool_button("refresh", "refresh", "Обновить", PANEL_ALT).on_click(cx.listener(|this, _, _, cx| this.refresh(cx))))
                    .child(tool_button("select-optional", "sparkles", "Рекомендуемые", PANEL_ALT).on_click(cx.listener(|this, _, _, cx| this.select_recommended(cx))))
                    .child(tool_button("clear-selection", "x", "Снять выбор", PANEL_ALT).on_click(cx.listener(|this, _, _, cx| {
                        this.selected.clear();
                        this.confirm_disable = false;
                        cx.notify();
                    })))
                    .child(
                        tool_button("show-all", "eye", if self.show_all { "Скрыть системные" } else { "Показать все" }, PANEL_ALT).on_click(cx.listener(|this, _, _, cx| {
                            this.show_all = !this.show_all;
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div().text_xs().text_color(MUTED).child(
                    "Зелёные — необязательные · оранжевые — проверить последствия · серые — защищены от отключения",
                ),
            )
            .child(list);

        if self.confirm_disable {
            page = page.child(
                card()
                    .border_color(RED)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(icon("warning", RED, px(22.)))
                            .child(label(&format!("Отключить выбранные пакеты: {installed_selected}?"))),
                    )
                    .child(div().text_xs().text_color(MUTED).child("Проверьте названия. Восстановление доступно в этом же списке: отключённые пакеты останутся видны."))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(tool_button("confirm-disable", "trash", "Подтвердить отключение", RED).on_click(cx.listener(|this, _, _, cx| this.change_packages(false, cx))))
                            .child(tool_button("cancel-disable", "x", "Отмена", PANEL_ALT).on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_disable = false;
                                cx.notify();
                            }))),
                    ),
            );
        } else {
            page = page.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        tool_button(
                            "disable-selected",
                            "trash",
                            format!("Отключить ({installed_selected})"),
                            RED,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.change_packages(false, cx))),
                    )
                    .child(
                        tool_button(
                            "restore-selected",
                            "restore",
                            format!("Восстановить ({removed_selected})"),
                            GREEN,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.change_packages(true, cx))),
                    ),
            );
        }
        page.into_any_element()
    }

    fn catalog_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.device.is_none() {
            return empty_state("Подключите телевизор, чтобы открыть каталог", cx)
                .into_any_element();
        }
        let mut page = div()
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(icon("download", ACCENT, px(28.)))
                            .child(title("Каталог приложений")),
                    )
                    .child(
                        tool_button("refresh-catalog", "refresh", "Обновить", PANEL_ALT)
                            .on_click(cx.listener(|this, _, _, cx| this.load_catalog(cx))),
                    ),
            )
            .child(subtitle(
                "Официальные релизы · выбор APK под архитектуру ТВ · проверка SHA-256",
            ));

        if self.catalog.is_empty() {
            page = page.child(
                card()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(label(if self.busy { "Загружаем каталог…" } else { "Каталог ещё не загружен" }))
                    .child(div().text_sm().text_color(MUTED).child("Версии и ссылки загружаются напрямую из официальных релизов разработчиков."))
                    .child(
                        tool_button("load-catalog", "download", "Загрузить каталог", ACCENT)
                            .on_click(cx.listener(|this, _, _, cx| this.load_catalog(cx))),
                    ),
            );
        } else {
            let mut list = div()
                .id("catalog-list")
                .w_full()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_wrap()
                .gap_3();
            for item in self.catalog.clone() {
                list = list.child(catalog_card(item, cx));
            }
            page = page.child(list);
        }
        page.into_any_element()
    }

    fn launcher_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let current = self
            .device
            .as_ref()
            .map(|device| device.launcher.as_str())
            .unwrap_or("телевизор не подключён");
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(icon("home", ACCENT, px(28.)))
                    .child(title("Замена главного экрана")),
            )
            .child(subtitle("Установка проверенного лаунчера без удаления резервного"))
            .child(
                card()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_4()
                            .child(
                                div()
                                    .size_16()
                                    .rounded_xl()
                                    .bg(ACCENT_SOFT)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon("home", ACCENT, px(36.))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(div().text_xl().font_weight(gpui::FontWeight::BOLD).child("Projectivy Launcher"))
                                    .child(div().text_sm().text_color(MUTED).child("Версия 4.73 · официальный релиз spocky/miproja1")),
                            ),
                    )
                    .child(label(&format!("Текущий лаунчер: {current}")))
                    .child(
                        div().text_sm().text_color(MUTED).child(
                            "SHA-256 проверяется автоматически. После установки Projectivy назначается главным и открывается Home. Старый лаунчер остаётся резервным.",
                        ),
                    )
                    .child(
                        div().text_sm().text_color(ORANGE).child(
                            "Проверьте Home, приложения и HDMI-входы. Только после этого отключайте старый лаунчер через раздел «Пакеты».",
                        ),
                    )
                    .child(tool_button("install-projectivy", "download", "Скачать, установить и сделать главным", ACCENT).on_click(cx.listener(|this, _, _, cx| this.install_projectivy(cx)))),
            )
            .into_any_element()
    }

    fn guide_page(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("guide")
            .w_full()
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(icon("terminal", ACCENT, px(28.)))
                    .child(title("Как включить ADB на телевизоре")),
            )
            .child(
                card().flex().flex_col().gap_3().child(section_label("tv", "Обычный Android TV / Google TV")).child(
                    "1. Откройте Настройки → О телевизоре.\n2. Нажмите 7 раз на «Номер сборки».\n3. Откройте «Для разработчиков».\n4. Включите «Отладка по USB», «ADB по сети» или «Сетевая отладка».\n5. Подключитесь к тому же Wi-Fi, что и компьютер.",
                ),
            )
            .child(
                card().flex().flex_col().gap_3().child(section_label("warning", "Если пункта отладки нет")).child(
                    "Производитель может скрыть ADB. Для некоторых CVTE/Яндекс ТВ: на главном экране зажмите Menu на 1–2 секунды, затем быстро Влево → Вверх → Влево → Вверх → Назад. В инженерном меню включите только ADB.\n\nНе меняйте Panel Type, LVDS, White Balance, Backlight и Shipping Init.",
                ),
            )
            .child(
                card().flex().flex_col().gap_3().child(section_label("wifi", "Подключение")).child(
                    "Найдите IP телевизора в настройках сети. Введите его в приложении и нажмите «Подключиться». На экране ТВ подтвердите запрос отладки и, при желании, выберите «Всегда разрешать с этого компьютера».",
                ),
            )
            .child(
                card().flex().flex_col().gap_3().child(section_label("lock", "Безопасность")).child(
                    "Не отключайте настройки, клавиатуру, системные службы и текущий лаунчер. После завершения работы выключите ADB на телевизоре: открытый ADB в общей Wi-Fi сети небезопасен.",
                ),
            )
            .into_any_element()
    }

    fn status_panel(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .border_t_1()
            .border_color(BORDER)
            .pt_3()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .text_color(if self.busy { ORANGE } else { TEXT })
                    .child(if self.busy {
                        format!("⏳ {}", self.status)
                    } else {
                        self.status.clone()
                    }),
            )
            .children(
                self.logs
                    .iter()
                    .rev()
                    .take(2)
                    .map(|line| div().text_xs().text_color(MUTED).child(line.clone())),
            )
    }
}

impl Render for TvAssistant {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.page {
            Page::Connection => self.connection_page(cx),
            Page::Packages => self.packages_page(cx),
            Page::Catalog => self.catalog_page(cx),
            Page::Launcher => self.launcher_page(cx),
            Page::Guide => self.guide_page(cx),
        };
        div()
            .size_full()
            .bg(BACKGROUND)
            .text_color(TEXT)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .child(self.navigation(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().flex_1().min_h_0().child(content))
                    .child(self.status_panel(cx)),
            )
    }
}

pub fn run(initial_device: Option<DeviceSnapshot>) {
    run_with_page(initial_device, Page::Packages);
}

pub fn run_catalog(initial_device: Option<DeviceSnapshot>) {
    run_with_page(initial_device, Page::Catalog);
}

fn run_with_page(initial_device: Option<DeviceSnapshot>, initial_page: Page) {
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(1280.), px(840.)), cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        window_min_size: Some(size(px(1100.), px(700.))),
                        app_id: Some("ru.tvassistant.desktop".to_string()),
                        titlebar: Some(TitlebarOptions {
                            title: Some("TV Assistant".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|cx| {
                            let mut view = TvAssistant::new(cx, initial_device);
                            if initial_page == Page::Catalog {
                                view.page = Page::Catalog;
                                view.load_catalog(cx);
                            }
                            view
                        })
                    },
                )
                .unwrap();
            window
                .update(cx, |view, window, cx| {
                    window.focus(&view.focus_handle);
                    cx.activate(true);
                })
                .unwrap();
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        });
}

fn catalog_card(item: CatalogItem, cx: &mut Context<TvAssistant>) -> gpui::Div {
    let id = item.id.clone();
    card()
        .w(px(430.))
        .h(px(205.))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .size_10()
                        .rounded_lg()
                        .bg(ACCENT_SOFT)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(&item.icon, ACCENT, px(23.))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(item.name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(MUTED)
                                .child(format!("{} · {}", item.category, item.version)),
                        ),
                )
                .when(item.installed, |card| {
                    card.child(badge("установлено", GREEN, GREEN_SOFT, px(96.)))
                }),
        )
        .child(
            div()
                .h(px(40.))
                .text_sm()
                .text_color(MUTED)
                .child(item.description.clone()),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .text_color(MUTED)
                        .child(format!("APK · {}", format_size(item.size))),
                )
                .child(
                    tool_button(
                        SharedString::from(format!("install-{id}")),
                        "download",
                        if item.installed {
                            "Переустановить"
                        } else {
                            "Установить"
                        },
                        if item.installed { PANEL_ALT } else { ACCENT },
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.install_catalog_item(id.clone(), cx)
                        }),
                    ),
                ),
        )
}

fn package_row(
    index: usize,
    package: &PackageInfo,
    selected: bool,
    view: gpui::Entity<TvAssistant>,
) -> gpui::Stateful<gpui::Div> {
    let name = package.name.clone();
    let protected = package.risk == Risk::Protected;
    let (risk_label, risk_color, risk_background) = match package.risk {
        Risk::Optional => ("необязательный", GREEN, GREEN_SOFT),
        Risk::Caution => ("осторожно", ORANGE, ORANGE_SOFT),
        Risk::Protected => ("защищён", MUTED, PANEL_ALT),
    };
    let (state_label, state_color, state_background) = if !package.installed {
        ("отключён", RED, RED_SOFT)
    } else if package.running {
        ("работает", ACCENT, ACCENT_SOFT)
    } else {
        ("установлен", GREEN, GREEN_SOFT)
    };
    div()
        .id(index)
        .h(px(68.))
        .w_full()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(BORDER)
        .bg(if selected {
            ACCENT_SOFT
        } else if index % 2 == 0 {
            PANEL
        } else {
            BACKGROUND
        })
        .when(!protected, |row| {
            row.cursor_pointer().on_click(move |_, _, cx| {
                view.update(cx, |this, cx| {
                    if this.selected.contains(&name) {
                        this.selected.remove(&name);
                    } else {
                        this.selected.insert(name.clone());
                    }
                    this.confirm_disable = false;
                    cx.notify();
                });
            })
        })
        .flex()
        .items_center()
        .gap_3()
        .child(if protected {
            icon("lock", MUTED, px(20.))
        } else if selected {
            icon("check", ACCENT, px(22.))
        } else {
            icon("square", MUTED, px(22.))
        })
        .child(icon("package", MUTED, px(21.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .truncate()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(package.label.clone()),
                )
                .child(div().text_xs().text_color(MUTED).truncate().child(
                    if package.third_party {
                        format!("{} · сторонний", package.name)
                    } else {
                        package.name.clone()
                    },
                )),
        )
        .child(badge(state_label, state_color, state_background, px(104.)))
        .child(badge(risk_label, risk_color, risk_background, px(132.)))
}

fn empty_state(message: &str, cx: &mut Context<TvAssistant>) -> impl IntoElement {
    card()
        .flex()
        .flex_col()
        .gap_4()
        .child(label(message))
        .child(
            action_button("empty-connect", "Перейти к подключению", ACCENT).on_click(cx.listener(
                |this, _, _, cx| {
                    this.page = Page::Connection;
                    cx.notify();
                },
            )),
        )
}

fn title(text: &str) -> gpui::Div {
    div()
        .text_2xl()
        .font_weight(gpui::FontWeight::BOLD)
        .child(text.to_string())
}

fn subtitle(text: &str) -> gpui::Div {
    div().text_sm().text_color(MUTED).child(text.to_string())
}

fn label(text: &str) -> gpui::Div {
    div()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(text.to_string())
}

fn section_label(icon_name: &'static str, text: &str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(icon(icon_name, ACCENT, px(19.)))
        .child(label(text))
}

fn card() -> gpui::Div {
    div()
        .bg(PANEL)
        .border_1()
        .border_color(BORDER)
        .rounded_lg()
        .p_4()
}

fn action_button(
    id: impl Into<ElementId>,
    label: impl IntoElement,
    color: Rgba,
) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .bg(color)
        .rounded_md()
        .px_3()
        .py_2()
        .cursor_pointer()
        .child(label)
        .hover(move |style| style.bg(color.blend(rgb(0x101010))))
}

fn icon(name: &str, color: Rgba, size: gpui::Pixels) -> Svg {
    svg().path(resource_path(name)).size(size).text_color(color)
}

fn icon_action_button(
    id: impl Into<ElementId>,
    icon_name: &'static str,
    label: impl IntoElement,
    color: Rgba,
) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .w_full()
        .bg(color)
        .rounded_md()
        .px_3()
        .py_2()
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_3()
        .child(icon(icon_name, TEXT, px(19.)))
        .child(label)
        .hover(move |style| style.bg(color.blend(rgb(0x101010))))
}

fn tool_button(
    id: impl Into<ElementId>,
    icon_name: &'static str,
    label: impl IntoElement,
    color: Rgba,
) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .bg(color)
        .rounded_md()
        .px_3()
        .py_2()
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .child(icon(icon_name, TEXT, px(17.)))
        .child(label)
        .hover(move |style| style.bg(color.blend(rgb(0x101010))))
}

fn badge(text: impl IntoElement, color: Rgba, background: Rgba, width: gpui::Pixels) -> gpui::Div {
    div()
        .w(width)
        .px_2()
        .py_1()
        .rounded_md()
        .bg(background)
        .text_color(color)
        .text_xs()
        .text_center()
        .child(text)
}

fn stat_card(icon_name: &'static str, caption: &str, value: &str) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .bg(PANEL)
        .border_1()
        .border_color(BORDER)
        .rounded_lg()
        .p_3()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .size_9()
                .rounded_md()
                .bg(ACCENT_SOFT)
                .flex()
                .items_center()
                .justify_center()
                .child(icon(icon_name, ACCENT, px(20.))),
        )
        .child(
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_xs().text_color(MUTED).child(caption.to_string()))
                .child(
                    div()
                        .truncate()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(value.to_string()),
                ),
        )
}
