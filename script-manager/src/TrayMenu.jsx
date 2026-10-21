import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  disable,
  enable,
  isEnabled,
} from "@tauri-apps/plugin-autostart";
import { CheckIcon, StopIcon, TerminalIcon, XIcon } from "./Icons";
import { applyTheme, getStoredTheme } from "./theme";
import logo from "./assets/logo.png";
import "./TrayMenu.css";

/** Custom tray context menu rendered in its own popup window, styled like the main app. */
function TrayMenu() {
  const [runningCount, setRunningCount] = useState(0);
  const [autostart, setAutostart] = useState(false);

  useEffect(() => {
    const menuWindow = getCurrentWindow();

    invoke("get_running_scripts").then((entries) =>
      setRunningCount(entries.length)
    );
    isEnabled().then(setAutostart);

    // Refresh the running count and theme each time the menu is opened.
    const unlistenFocus = menuWindow.onFocusChanged(({ payload: focused }) => {
      if (focused) {
        applyTheme(getStoredTheme());
        invoke("get_running_scripts").then((entries) =>
          setRunningCount(entries.length)
        );
        isEnabled().then(setAutostart);
      }
    });

    // Live-sync the theme when the main window changes it.
    function handleStorage(event) {
      if (event.key === "theme") {
        applyTheme(getStoredTheme());
      }
    }

    window.addEventListener("storage", handleStorage);

    return () => {
      unlistenFocus.then((stop) => stop());
      window.removeEventListener("storage", handleStorage);
    };
  }, []);

  /** Hides the menu popup, then runs the chosen action. */
  async function runAction(action) {
    await getCurrentWindow().hide();
    await action();
  }

  /** Toggles launching the app at Windows login; the menu stays open
      so the check mark gives immediate feedback. */
  async function toggleAutostart() {
    if (autostart) {
      await disable();
    } else {
      await enable();
    }

    setAutostart(await isEnabled());
  }

  return (
    <div className="tray-menu">
      <div className="tray-menu-header">
        <span className="tray-menu-brand">
          <img
            className="tray-menu-logo"
            src={logo}
            alt=""
            aria-hidden="true"
          />
          <span className="tray-menu-title">脚本管理器</span>
        </span>
        <span
          className={`tray-menu-status ${runningCount > 0 ? "active" : ""}`}
        >
          {runningCount} 个运行中
        </span>
      </div>

      <button
        className="tray-menu-item"
        onClick={() => runAction(() => invoke("toggle_main"))}
      >
        <TerminalIcon size={14} />
        打开脚本管理器
      </button>

      <button
        className="tray-menu-item"
        disabled={runningCount === 0}
        onClick={() => runAction(() => invoke("stop_all_scripts"))}
      >
        <StopIcon size={14} />
        停止全部脚本
      </button>

      <button className="tray-menu-item" onClick={toggleAutostart}>
        <span className={`tray-menu-check ${autostart ? "checked" : ""}`}>
          {autostart && <CheckIcon size={12} />}
        </span>
        开机启动
      </button>

      <div className="tray-menu-separator" />

      <button
        className="tray-menu-item danger"
        onClick={() => runAction(() => invoke("quit_app"))}
      >
        <XIcon size={14} />
        退出
      </button>
    </div>
  );
}

export default TrayMenu;
