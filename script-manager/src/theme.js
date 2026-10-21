/** Theme persistence and application, shared by every window of the app. */

const THEME_KEY = "theme";

/** Returns the persisted theme, defaulting to dark. */
export function getStoredTheme() {
  return localStorage.getItem(THEME_KEY) === "light" ? "light" : "dark";
}

/** Applies a theme to the document without persisting it. */
export function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
}

/** Persists a theme and applies it; other windows sync via the storage event. */
export function storeTheme(theme) {
  localStorage.setItem(THEME_KEY, theme);
  applyTheme(theme);
}
