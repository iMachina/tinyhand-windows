//! `--e2e-test`: turns the real pointer on and drives it with synthetic mouse and keyboard
//! input, checking that taps are swallowed, drags draw, and real clicks get through when asked.
//!
//! It takes over the mouse for a few seconds. A small helper window from a second process
//! (`--e2e-target`) sits under the cursor and records the clicks it actually receives.

use crate::{app, hook, prefs};
use std::io::Write;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        app::pump_once(Some(5));
    }
}

fn mouse(flags: MOUSE_EVENT_FLAGS) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: flags, ..Default::default() } },
    };
    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

/// Moves the mouse with real input events, so the hooks see the movement.
fn move_to(x: i32, y: i32) {
    unsafe {
        let (vx, vy) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        let (vw, vh) = (GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1), GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1));
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: (((x - vx) as i64 * 65535) / (vw as i64 - 1).max(1)) as i32,
                    dy: (((y - vy) as i64 * 65535) / (vh as i64 - 1).max(1)) as i32,
                    dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    ..Default::default()
                },
            },
        };
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn key(vk: VIRTUAL_KEY, up: bool) {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, ..Default::default() },
        },
    };
    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn click() {
    mouse(MOUSEEVENTF_LEFTDOWN);
    pump(60);
    mouse(MOUSEEVENTF_LEFTUP);
    pump(120);
}

/// Sum of the colour values in a small square of the screen, to tell whether it changed.
fn screen_sum(x: i32, y: i32, side: i32) -> u64 {
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, side, side);
        let old = SelectObject(mem, bitmap.into());
        let _ = BitBlt(mem, 0, 0, side, side, Some(screen), x, y, SRCCOPY | CAPTUREBLT);
        let mut sum = 0u64;
        for py in (0..side).step_by(2) {
            for px in (0..side).step_by(2) {
                let c = GetPixel(mem, px, py).0;
                sum += (c & 0xFF) as u64 + ((c >> 8) & 0xFF) as u64 + ((c >> 16) & 0xFF) as u64;
            }
        }
        SelectObject(mem, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        sum
    }
}

/// Saves a picture of part of the screen, to check by eye that the overlay is transparent.
fn screenshot(x: i32, y: i32, w: i32, h: i32, path: &str) {
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bitmap.into());
        let _ = BitBlt(mem, 0, 0, w, h, Some(screen), x, y, SRCCOPY | CAPTUREBLT);
        let mut image = image::RgbImage::new(w as u32, h as u32);
        for py in 0..h {
            for px in 0..w {
                let c = GetPixel(mem, px, py).0;
                image.put_pixel(px as u32, py as u32, image::Rgb([c as u8, (c >> 8) as u8, (c >> 16) as u8]));
            }
        }
        SelectObject(mem, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        let _ = image.save(path);
    }
}

fn target_log(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path).unwrap_or_default().lines().map(str::to_string).collect()
}

struct Report {
    lines: Vec<String>,
    failed: usize,
}

impl Report {
    fn check(&mut self, name: &str, ok: bool, detail: String) {
        if !ok {
            self.failed += 1;
        }
        self.lines.push(format!("{} {name}  [{detail}]", if ok { "PASS" } else { "FAIL" }));
    }
}

/// Runs the whole test and writes the results to `out`. Returns the number of failures.
pub fn run(out: &str) -> i32 {
    let dir = std::env::temp_dir().join(format!("tinyhand-e2e-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    // Use throwaway settings so the test neither depends on nor changes the user's.
    std::env::set_var("TH_DATA_DIR", dir.join("data"));
    let _ = std::fs::create_dir_all(dir.join("data"));
    let _ = std::fs::write(dir.join("data").join("settings.json"), r#"{"welcomeShown":true,"handSound":false}"#);
    let log = dir.join("target.log");
    let mut report = Report { lines: Vec::new(), failed: 0 };

    // Never hold the user's mouse hostage: if the test hangs, end the process. The cursor
    // and the hooks are released by Windows when it goes.
    let trace = format!("{out}.trace");
    let _ = std::fs::write(&trace, "");
    let step = move |name: &str| {
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&trace) {
            let _ = writeln!(f, "{name}");
        }
    };
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(40));
        std::process::exit(99);
    });
    step("start");

    let mut start = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut start);
    }
    // The target window goes near the middle of the monitor the mouse is on.
    let (tx, ty) = unsafe {
        let monitor = MonitorFromPoint(start, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        ((info.rcWork.left + info.rcWork.right) / 2 - 200, (info.rcWork.top + info.rcWork.bottom) / 2 - 150)
    };
    let exe = std::env::current_exe().unwrap_or_default();
    let child = std::process::Command::new(exe)
        .args(["--e2e-target", &log.to_string_lossy(), &tx.to_string(), &ty.to_string()])
        .spawn();
    let Ok(mut child) = child else {
        let _ = std::fs::write(out, "FAIL could not start the target window\n");
        return 1;
    };
    let waited = Instant::now();
    while target_log(&log).is_empty() && waited.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }

    step("target ready");
    if !app::init() {
        let _ = child.kill();
        let _ = std::fs::write(out, "FAIL app::init\n");
        return 1;
    }
    step("init done");
    pump(50);
    let (cx, cy) = (tx + 200, ty + 150);

    // 1. The overlay shows the hand.
    move_to(cx, cy);
    pump(100);
    step("moved");
    let before = screen_sum(cx - 40, cy - 10, 160);
    step("captured");
    app::with(|a| a.activate());
    pump(700);
    let active = app::with(|a| a.is_active()).unwrap_or(false);
    report.check("pointer turns on", active, format!("active={active} hooks={}", hook::is_installed()));
    let during = screen_sum(cx - 40, cy - 10, 160);
    report.check("hand is visible on screen", before != during, format!("pixels before={before} with hand={during}"));

    step("activated");
    // 2. A click is a tap: the window underneath never hears about it.
    click();
    let got = target_log(&log);
    report.check("click is swallowed as a tap", got.len() == 1, format!("target saw {:?}", &got[1.min(got.len())..]));

    step("tapped");
    // 3. Hold and drag draws one line.
    mouse(MOUSEEVENTF_LEFTDOWN);
    pump(40);
    let frames_before = app::with(|a| a.frames).unwrap_or(0);
    let sweep = Instant::now();
    for i in 1..=120 {
        move_to(cx + i, cy + (i % 5) * 4);
        pump(8);
    }
    screenshot(tx - 60, ty - 60, 620, 520, &format!("{out}.png"));
    let fps = (app::with(|a| a.frames).unwrap_or(0) - frames_before) as f64 / sweep.elapsed().as_secs_f64();
    report.lines.push(format!("INFO {fps:.0} frames a second while the mouse moves"));
    let drawing = app::with(|a| a.is_drawing()).unwrap_or(false);
    mouse(MOUSEEVENTF_LEFTUP);
    pump(120);
    let strokes = app::with(|a| a.ink_strokes()).unwrap_or(0);
    let got = target_log(&log);
    report.check(
        "drag draws a line",
        drawing && strokes == 1 && got.len() == 1,
        format!("drawing={drawing} strokes={strokes} target events={}", got.len() - 1),
    );
    move_to(cx, cy);
    pump(60);

    step("dragged");
    // 4. With the interact key held, the click reaches the app as a plain click.
    key(VK_LMENU, false);
    pump(80);
    click();
    key(VK_LMENU, true);
    pump(120);
    let got = target_log(&log);
    let plain = got[1.min(got.len())..] == ["down alt=0".to_string(), "up".to_string()];
    report.check("Alt+click reaches the app without Alt", plain, format!("target saw {:?}", &got[1..]));
    let alt_stuck = unsafe { (GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000) != 0 };
    report.check("Alt is not left stuck down", !alt_stuck, format!("alt down={alt_stuck}"));

    step("alt-clicked");
    // 5. Double-tap Shift latches click-through; again turns it off.
    let n = got.len();
    for _ in 0..2 {
        key(VK_LSHIFT, false);
        pump(40);
        key(VK_LSHIFT, true);
        pump(80);
    }
    pump(100);
    let through = app::with(|a| a.is_click_through()).unwrap_or(false);
    click();
    let got = target_log(&log);
    report.check(
        "double-tap Shift lets clicks through",
        through && got.len() == n + 2,
        format!("click-through={through} new target events={}", got.len() - n),
    );
    pump(500);
    for _ in 0..2 {
        key(VK_LSHIFT, false);
        pump(40);
        key(VK_LSHIFT, true);
        pump(80);
    }
    pump(100);
    let through = app::with(|a| a.is_click_through()).unwrap_or(true);
    report.check("double-tap Shift again goes back to taps", !through, format!("click-through={through}"));

    step("shifted");
    // 6. The toolbar's last button (the X) turns the pointer off without clicking through.
    if let Some(frame) = app::with(|a| a.toolbar_frame()).flatten() {
        let h = frame.bottom - frame.top;
        move_to(frame.right - h / 2, frame.top + h / 2);
        pump(400);
        click();
        let active = app::with(|a| a.is_active()).unwrap_or(true);
        report.check("toolbar exit button turns the pointer off", !active, format!("active={active}"));
    } else {
        report.check("toolbar exit button turns the pointer off", false, "no toolbar frame".into());
    }

    step("toolbar");
    // 7. The global shortcut turns it back on, Esc turns it off.
    move_to(cx, cy);
    pump(60);
    key(VK_CONTROL, false);
    key(VK_MENU, false);
    key(VIRTUAL_KEY(b'H' as u16), false);
    key(VIRTUAL_KEY(b'H' as u16), true);
    key(VK_MENU, true);
    key(VK_CONTROL, true);
    pump(400);
    let active = app::with(|a| a.is_active()).unwrap_or(false);
    report.check("Ctrl+Alt+H turns the pointer on", active, format!("active={active}"));
    key(VK_ESCAPE, false);
    key(VK_ESCAPE, true);
    pump(300);
    let active = app::with(|a| a.is_active()).unwrap_or(true);
    report.check("Esc turns the pointer off", !active, format!("active={active}"));
    pump(200);
    let after = screen_sum(cx - 40, cy - 10, 160);
    report.check("hand is gone from the screen", after == before, format!("pixels before={before} after={after}"));

    step("hotkeys");
    // 8. The settings window opens, switches page and changes a setting with real clicks.
    step("settings");
    crate::settings::show();
    pump(600);
    let window = unsafe { FindWindowW(None, w!("TinyHand Settings")).unwrap_or_default() };
    if window.0.is_null() {
        report.check("settings window opens", false, "window not found".into());
    } else {
        let (mut origin, mut rc) = (POINT::default(), RECT::default());
        let scale = unsafe {
            let _ = ClientToScreen(window, &mut origin);
            let _ = GetClientRect(window, &mut rc);
            windows::Win32::UI::HiDpi::GetDpiForWindow(window) as f32 / 96.0
        };
        let at = |x: f32, y: f32| (origin.x + (x * scale) as i32, origin.y + (y * scale) as i32);
        let before = prefs::get().hand_sound;
        let (x, y) = at(75.0, 211.0); // "Sound" in the sidebar
        move_to(x, y);
        pump(150);
        click();
        let (x, y) = at(695.0, 147.0); // the "Tap sound" switch
        move_to(x, y);
        pump(150);
        click();
        pump(300);
        let after = prefs::get().hand_sound;
        screenshot(origin.x, origin.y, rc.right, rc.bottom, &format!("{out}.settings.png"));
        report.check("settings window opens and a switch works", before != after, format!("tap sound {before} -> {after}"));
        unsafe {
            let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        pump(300);
        let closed = unsafe { !IsWindow(Some(window)).as_bool() || !IsWindowVisible(window).as_bool() };
        report.check("settings window closes", closed, format!("closed={closed}"));
    }

    // Tidy up: everything off, keys up, mouse back where it was.
    app::with(|a| a.deactivate());
    for vk in [VK_LMENU, VK_MENU, VK_CONTROL, VK_LSHIFT] {
        key(vk, true);
    }
    app::shutdown();
    unsafe {
        let _ = SetCursorPos(start.x, start.y);
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::copy(dir.join("data").join("tinyhand.log"), format!("{out}.log"));
    let _ = std::fs::remove_dir_all(&dir);

    let checks = report.lines.iter().filter(|l| !l.starts_with("INFO")).count();
    report.lines.push(format!("{checks} checks, {} failed", report.failed));
    let _ = std::fs::write(out, report.lines.join("\n") + "\n");
    report.failed as i32
}

// ---- The helper window ---------------------------------------------------------------

thread_local! {
    static LOG: std::cell::RefCell<Option<std::fs::File>> = const { std::cell::RefCell::new(None) };
}

fn note(line: &str) {
    LOG.with(|l| {
        if let Some(f) = l.borrow_mut().as_mut() {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    });
}

unsafe extern "system" fn target_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => {
            let alt = (GetKeyState(VK_MENU.0 as i32) as u16 & 0x8000) != 0;
            note(&format!("down alt={}", alt as u8));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            note("up");
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// A plain topmost window that writes every click it receives to `log`.
pub fn target(log: &str, x: i32, y: i32) -> i32 {
    LOG.with(|l| *l.borrow_mut() = std::fs::File::create(log).ok());
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(target_proc),
            lpszClassName: w!("TinyHandTestTarget"),
            hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&class);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.lpszClassName,
            w!("TinyHand test target"),
            WS_POPUP | WS_VISIBLE | WS_BORDER,
            x,
            y,
            400,
            300,
            None,
            None,
            None,
            None,
        ) else {
            return 1;
        };
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        note("ready");
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    0
}
