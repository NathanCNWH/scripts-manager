import { getCurrentWindow } from "@tauri-apps/api/window";
import { MoonIcon, SunIcon, XIcon } from "./Icons";
import logo from "./assets/logo.png";
import "./TitleBar.css";

/** Custom title bar replacing the native frame: app title, theme toggle,
    and a close button that hides the window to the tray. */
function TitleBar({ theme, onToggleTheme, onError }) {
  async function hideWindow() {
    await getCurrentWindow().hide();
  }

  return (
    <header className="title-bar" onMouseDown={async (event) => {
      if (event.button !== 0 || event.target.closest("button")) return;
      try {
        await getCurrentWindow().startDragging();
      } catch (message) {
        onError(`无法移动窗口：${message}`);
      }
    }}>
      <div className="title-bar-brand">
        <img className="title-bar-logo" src={logo} alt="" aria-hidden="true" />
        <span className="title-bar-label">脚本管理器</span>
      </div>

      <div className="title-bar-actions">
        <button
          className="title-bar-button"
          onClick={onToggleTheme}
          aria-label={
            theme === "dark" ? "切换为浅色主题" : "切换为深色主题"
          }
          title={theme === "dark" ? "浅色主题" : "深色主题"}
        >
          {theme === "dark" ? <SunIcon size={15} /> : <MoonIcon size={15} />}
        </button>

        <button
          className="title-bar-button close"
          onClick={hideWindow}
          aria-label="隐藏窗口"
          title="隐藏到托盘"
        >
          <XIcon size={15} />
        </button>
      </div>
    </header>
  );
}

export default TitleBar;
