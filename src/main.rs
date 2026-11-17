//! TinyHand for Windows: a tiny hand on a stick that points at things on your screen.
//!
//! A Rust port of the macOS app at https://github.com/nilbuild/tinyhand (MIT).

#![windows_subsystem = "windows"]

mod app;
mod arm;
mod automation;
mod cursor;
mod custom;
mod devtest;
mod e2e;
mod handasset;
mod handmaker;
mod hook;
mod ink;
mod log;
mod paths;
mod prefs;
mod registry;
mod render;
mod settings;
mod sound;
mod styles;
mod toolbar;

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, LPARAM, WPARAM};
use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SendMessageW, WM_COPYDATA};

/// Hands a command to the copy of TinyHand that is already running.
fn forward(command: &str) -> bool {
    unsafe {
        let Ok(target) = FindWindowW(app::MAIN_CLASS, None) else { return false };
        if target.0.is_null() {
            return false;
        }
        let data = COPYDATASTRUCT {
            dwData: app::COPYDATA_COMMAND,
            cbData: command.len() as u32,
            lpData: command.as_ptr() as *mut _,
        };
        SendMessageW(target, WM_COPYDATA, Some(WPARAM(0)), Some(LPARAM(&data as *const _ as isize)));
        true
    }
}

/// Command-line tools that run and exit without starting the app. Returns an exit code
/// if one of them handled the arguments.
fn run_tool(args: &[String]) -> Option<i32> {
    let flag = args.first()?.as_str();
    if !flag.starts_with("--") {
        return None;
    }
    unsafe {
        // A GUI program has no console of its own; borrow the one it was started from.
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
    let arg = |i: usize| args.get(i).map(String::as_str);
    Some(match flag {
        "--render-test" => {
            let out = arg(1).unwrap_or("tinyhand-render.png");
            let style = arg(2).unwrap_or("lavender");
            let width = arg(3).and_then(|v| v.parse().ok()).unwrap_or(1600);
            let height = arg(4).and_then(|v| v.parse().ok()).unwrap_or(900);
            match devtest::render(out, style, width, height) {
                Ok(()) => {
                    println!("rendered {out}");
                    0
                }
                Err(e) => {
                    println!("failed: {e}");
                    1
                }
            }
        }
        "--make-hand" => match (arg(1), arg(2)) {
            (Some(input), Some(output)) => match handmaker::make(std::path::Path::new(input)) {
                Ok(made) => match made.image.save(output) {
                    Ok(()) => {
                        println!("ok {:06X}", made.ink);
                        0
                    }
                    Err(e) => {
                        println!("failed: {e}");
                        1
                    }
                },
                Err(problem) => {
                    println!("rejected: {problem}");
                    1
                }
            },
            _ => {
                println!("usage: tinyhand --make-hand <input.png> <output.png>");
                2
            }
        },
        "--e2e-test" => e2e::run(arg(1).unwrap_or("tinyhand-e2e.txt")),
        "--e2e-target" => {
            let coord = |i: usize| arg(i).and_then(|v| v.parse().ok()).unwrap_or(100);
            e2e::target(arg(1).unwrap_or("tinyhand-e2e-target.log"), coord(2), coord(3))
        }
        "--settings-test" => {
            let out = arg(1).unwrap_or("tinyhand-settings.png");
            let section = arg(2).unwrap_or("general");
            match settings::render_test(out, section, arg(3) == Some("dark")) {
                Ok(()) => {
                    println!("rendered {out}");
                    0
                }
                Err(e) => {
                    println!("failed: {e}");
                    1
                }
            }
        }
        "--version" => {
            println!("TinyHand {}", env!("CARGO_PKG_VERSION"));
            0
        }
        _ => {
            println!("unknown option {flag}");
            2
        }
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    if let Some(code) = run_tool(&args) {
        std::process::exit(code);
    }
    let command = args.first().cloned();

    // Only one TinyHand runs at a time. A second copy passes its command on and leaves.
    let already_running = unsafe {
        let _mutex = CreateMutexW(None, false, w!("Local\\TinyHand.SingleInstance"));
        GetLastError() == ERROR_ALREADY_EXISTS
    };
    if already_running {
        forward(command.as_deref().unwrap_or("settings"));
        return;
    }

    if !app::init() {
        std::process::exit(1);
    }
    registry::register_url_protocol();
    if let Some(command) = command {
        app::with(|app| automation::handle(app, &command));
    }
    app::run();
}
