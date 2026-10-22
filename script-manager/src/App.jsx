import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, open } from "@tauri-apps/plugin-dialog";
import TitleBar from "./TitleBar";
import {
  EditIcon,
  FileCodeIcon,
  PauseIcon,
  PlayIcon,
  PlusIcon,
  StopIcon,
  TerminalIcon,
  TrashIcon,
} from "./Icons";
import { getStoredTheme, storeTheme } from "./theme";
import "./App.css";
import { isPendingState, isRunningState, statusLabel } from "./runtime";

/** Cap on how many terminal lines are kept in memory. */
const MAX_LOG_LINES = 500;
const SCRIPT_FILTERS = [{
  name: "脚本文件",
  extensions: ["py", "ps1", "bat", "cmd", "js", "sh", "exe"],
}];

function emptyEntry() {
  return {
    name: "",
    mode: "file",
    path: "",
    command: "",
    shell: "cmd",
    working_dir: "",
    stopMode: "none",
    stopPath: "",
    stopCommand: "",
    stopShell: "cmd",
    monitorMode: "process",
    monitorHost: "127.0.0.1",
    monitorPort: "",
  };
}

/** Appends a raw output chunk to the line list. Chunks are not line-aligned:
    a line without a trailing newline stays "open" (a pending prompt) and is
    completed by the next chunk — or by the user's own input echo. */
function appendChunk(lines, id, stream, chunk) {
  const next = [...lines];
  const segments = chunk.split("\n");
  const endsWithNewline = segments[segments.length - 1] === "";

  if (endsWithNewline) {
    segments.pop();
  }

  if (segments.length === 0) {
    // The chunk was just a newline: close the pending line if any.
    const last = next[next.length - 1];

    if (last && last.open && last.id === id) {
      next[next.length - 1] = { ...last, open: false };
    }

    return next;
  }

  segments.forEach((rawSegment, index) => {
    const segment = rawSegment.endsWith("\r")
      ? rawSegment.slice(0, -1)
      : rawSegment;
    const isLastSegment = index === segments.length - 1;
    const open = isLastSegment && !endsWithNewline;
    const last = next[next.length - 1];

    if (index === 0 && last && last.open && last.id === id) {
      // Continue the pending line (e.g. prompt + the user's reply).
      next[next.length - 1] = { ...last, line: last.line + segment, open };
    } else {
      next.push({ id, stream, line: segment, open });
    }
  });

  return next.slice(-MAX_LOG_LINES);
}

/** Root view: script list with run/pause/stop controls and a toggleable output terminal. */
function App() {
  const [scripts, setScripts] = useState([]);
  const [runtimeStates, setRuntimeStates] = useState([]);
  const [busyIds, setBusyIds] = useState([]);
  const runningIds = runtimeStates.filter(isRunningState).map((entry) => entry.id);
  const pausedIds = runtimeStates.filter((entry) => entry.status === "paused").map((entry) => entry.id);
  const [logLines, setLogLines] = useState([]);
  const [showTerminal, setShowTerminal] = useState(false);
  const [selectedId, setSelectedId] = useState(null);
  const [theme, setTheme] = useState(getStoredTheme);
  const [error, setError] = useState("");
  const [contextMenu, setContextMenu] = useState(null);
  const [isDragging, setIsDragging] = useState(false);
  const [inputText, setInputText] = useState("");
  const [showAddForm, setShowAddForm] = useState(false);
  const [editingId, setEditingId] = useState(null);
  const [draft, setDraft] = useState(emptyEntry);
  const [configPath, setConfigPath] = useState("");
  const [configError, setConfigError] = useState("");
  const terminalRef = useRef(null);
  const inputRef = useRef(null);
  const contentRef = useRef(null);

  async function refreshStates() {
    setRuntimeStates(await invoke("get_script_states"));
  }

  function runtimeFor(script) {
    return runtimeStates.find((entry) => entry.id === script?.id) || {
      status: script?.monitor ? "checking" : "stopped",
      monitored: Boolean(script?.monitor),
      can_stop: Boolean(script?.stop),
      message: "",
    };
  }

  useEffect(() => {
    function refreshScripts() {
      invoke("get_scripts")
        .then((entries) => {
          setScripts(entries);
          setConfigError("");
          return refreshStates();
        })
        .catch((message) => setConfigError(String(message)));
    }
    refreshScripts();
    invoke("get_config_path")
      .then(setConfigPath)
      .catch((message) => setConfigError(String(message)));
    const refreshTimer = window.setInterval(refreshScripts, 2000);
    const unlistenStates = listen("script-states", (event) => setRuntimeStates(event.payload));

    const unlistenOutput = listen("script-output", (event) => {
      const { id, stream, chunk } = event.payload;

      setLogLines((lines) => appendChunk(lines, id, stream, chunk));
    });

    const unlistenExited = listen("script-exited", (event) => {
      const { id, code, monitored } = event.payload;
      refreshStates().catch((message) => setConfigError(String(message)));
      setLogLines((lines) =>
        [
          ...lines,
          {
            id,
            stream: "info",
            line: monitored
              ? `[启动脚本已结束，退出码：${code ?? "未知"}；继续检测服务端口]`
              : `[进程已退出，退出码：${code ?? "未知"}]`,
            open: false,
          },
        ].slice(-MAX_LOG_LINES)
      );
    });

    // Native file drag-and-drop from the OS (the webview cannot read real
    // paths from HTML5 drop events, so this goes through Tauri's events).
    // Listened globally rather than via onDragDropEvent, whose per-webview
    // target scoping can miss the events on Windows.
    const unlistenDragEnter = listen("tauri://drag-enter", () => {
      setIsDragging(true);
    });

    const unlistenDragLeave = listen("tauri://drag-leave", () => {
      setIsDragging(false);
    });

    const unlistenDragDrop = listen("tauri://drag-drop", (event) => {
      setIsDragging(false);
      addScriptPaths(event.payload.paths);
    });

    return () => {
      unlistenOutput.then((unlisten) => unlisten());
      unlistenStates.then((unlisten) => unlisten());
      unlistenExited.then((unlisten) => unlisten());
      unlistenDragEnter.then((unlisten) => unlisten());
      unlistenDragLeave.then((unlisten) => unlisten());
      unlistenDragDrop.then((unlisten) => unlisten());
      window.clearInterval(refreshTimer);
    };
  }, []);

  // The custom context menu replaces the webview's default one and closes
  // on any click, Escape, or a right-click elsewhere.
  useEffect(() => {
    function handleWindowContextMenu(event) {
      event.preventDefault();
      setContextMenu(null);
    }

    function handleClick() {
      setContextMenu(null);
    }

    function handleKeyDown(event) {
      if (event.key === "Escape") {
        setContextMenu(null);
      }
    }

    window.addEventListener("contextmenu", handleWindowContextMenu);
    window.addEventListener("click", handleClick);
    window.addEventListener("keydown", handleKeyDown);

    return () => {
      window.removeEventListener("contextmenu", handleWindowContextMenu);
      window.removeEventListener("click", handleClick);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, []);

  useEffect(() => {
    const terminal = terminalRef.current;

    if (terminal) {
      terminal.scrollTop = terminal.scrollHeight;
    }
  }, [logLines, showTerminal]);

  /** Switches between the dark and light theme and persists the choice. */
  function toggleTheme() {
    const next = theme === "dark" ? "light" : "dark";

    storeTheme(next);
    setTheme(next);
  }

  /** Registers one or more scripts by path, accumulating any errors. */
  async function addScriptPaths(paths) {
    setError("");

    const errors = [];

    for (const path of paths) {
      try {
        setScripts(await invoke("add_script", { path }));
      } catch (message) {
        errors.push(String(message));
      }
    }

    if (errors.length > 0) {
      setError(errors.join(" "));
    }
  }

  /** Opens a native file picker and registers the selected script. */
  async function importScript() {
    const selected = await open({
      title: "选择脚本文件",
      filters: SCRIPT_FILTERS,
    });

    if (selected) {
      await addScriptPaths([selected]);
    }
  }

  async function pickEntryFile(field) {
    try {
      const selected = await open({
        title: field === "path" ? "选择启动文件" : "选择停止文件",
        filters: SCRIPT_FILTERS,
      });
      if (selected) {
        setDraft((current) => ({ ...current, [field]: selected }));
      }
    } catch (message) {
      setError(String(message));
    }
  }

  function beginAdd() {
    setEditingId(null);
    setDraft(emptyEntry());
    setShowAddForm(true);
    setError("");
    contentRef.current?.scrollTo({ top: 0, behavior: "smooth" });
  }

  function beginEdit(id) {
    const script = scripts.find((item) => item.id === id);
    if (!script) {
      setError("此条目已不存在。");
      return;
    }
    const stopMode = script.stop?.command
      ? "command"
      : script.stop?.path
        ? "file"
        : "none";
    setDraft({
      name: script.name,
      mode: script.mode || "file",
      path: script.path || "",
      command: script.command || "",
      shell: script.shell || "cmd",
      working_dir: script.working_dir || "",
      stopMode,
      stopPath: script.stop?.path || "",
      stopCommand: script.stop?.command || "",
      stopShell: script.stop?.shell || "cmd",
      monitorMode: script.monitor ? "tcp" : "process",
      monitorHost: script.monitor?.host || "127.0.0.1",
      monitorPort: script.monitor ? String(script.monitor.port) : "",
    });
    setEditingId(id);
    setShowAddForm(true);
    setError("");
    contentRef.current?.scrollTo({ top: 0, behavior: "smooth" });
  }

  function cancelEntry() {
    setShowAddForm(false);
    setEditingId(null);
    setDraft(emptyEntry());
  }

  async function saveEntry(event) {
    event.preventDefault();
    setError("");
    const entry = {
      id: editingId || "",
      name: draft.name,
      mode: draft.mode,
      path: draft.mode === "file" ? draft.path : "",
      command: draft.mode === "command" ? draft.command : "",
      shell: draft.mode === "command" ? draft.shell : "",
      working_dir: draft.working_dir,
      monitor: draft.monitorMode === "tcp"
        ? { kind: "tcp", host: draft.monitorHost, port: Number(draft.monitorPort) }
        : null,
      stop: draft.stopMode === "none"
        ? null
        : {
            path: draft.stopMode === "file" ? draft.stopPath : "",
            command: draft.stopMode === "command" ? draft.stopCommand : "",
            shell: draft.stopMode === "command" ? draft.stopShell : "",
          },
    };
    try {
      const command = editingId ? "update_entry" : "add_entry";
      const args = editingId ? { id: editingId, entry } : { entry };
      setScripts(await invoke(command, args));
      await refreshStates();
      cancelEntry();
    } catch (message) {
      setError(String(message));
    }
  }

  async function openConfig() {
    setError("");
    try {
      await invoke("open_config_file");
      setConfigPath(await invoke("get_config_path"));
    } catch (message) {
      setError(String(message));
    }
  }

  /** Removes a script from the registry, asking for confirmation first
      when it is still running (removal kills the process). */
  async function removeScript(id) {
    setError("");

    if (runningIds.includes(id)) {
      const confirmed = await ask(
        `${scriptName(id)} 仍在运行，移除此条目将停止脚本。是否继续？`,
        { title: "移除运行中的脚本", kind: "warning", okLabel: "移除并停止", cancelLabel: "取消" }
      );

      if (!confirmed) {
        return;
      }
    }

    try {
      setScripts(await invoke("remove_script", { id }));
      setSelectedId((current) => (current === id ? null : current));
      if (editingId === id) {
        cancelEntry();
      }
    } catch (message) {
      setError(String(message));
    }
  }

  /** Starts a script and marks it as running. */
  async function startScript(id) {
    await controlScript("start_script", id);
  }

  /** Stops a managed process or directly invokes its configured stop action. */
  async function stopScript(id) {
    await controlScript("stop_script", id);
  }

  /** Freezes a running script's process. */
  async function pauseScript(id) {
    await controlScript("pause_script", id);
  }

  /** Unfreezes a paused script's process. */
  async function resumeScript(id) {
    await controlScript("resume_script", id);
  }

  async function controlScript(command, id) {
    setError("");
    setBusyIds((ids) => [...ids, id]);
    try {
      await invoke(command, { id });
    } catch (message) {
      setError(String(message));
    } finally {
      try { await refreshStates(); } catch (message) { setConfigError(String(message)); }
      setBusyIds((ids) => ids.filter((item) => item !== id));
    }
  }

  /** Sends the typed line to the selected running script's stdin. */
  async function sendInput() {
    if (!selectedId || !inputText || scripts.find((script) => script.id === selectedId)?.monitor) {
      return;
    }

    setError("");

    try {
      await invoke("send_input", { id: selectedId, text: inputText });
      setInputText("");
    } catch (message) {
      setError(String(message));
    }
  }

  /** Resolves a script id to its display name for terminal prefixes. */
  function scriptName(id) {
    const script = scripts.find((entry) => entry.id === id);
    return script ? script.name : id;
  }

  /** Opens a script's terminal, switches scripts, or closes the active terminal. */
  function toggleSelection(id) {
    if (showTerminal && selectedId === id) {
      setShowTerminal(false);
      setSelectedId(null);
      return;
    }
    setSelectedId(id);
    setShowTerminal(true);
  }

  /** Wraps a row-action handler so it does not change the selection. */
  function rowAction(handler, id) {
    return (event) => {
      event.stopPropagation();
      handler(id);
    };
  }

  const visibleLines = selectedId
    ? logLines.filter((entry) => entry.id === selectedId)
    : logLines;

  const lastVisible = visibleLines[visibleLines.length - 1];
  const hasPendingPrompt = Boolean(lastVisible && lastVisible.open);

  // A pending prompt on the selected running script focuses the input, so
  // answering an input() request is just: see it appear, type, Enter.
  useEffect(() => {
    if (
      hasPendingPrompt &&
      showTerminal &&
      selectedId &&
      runningIds.includes(selectedId)
    ) {
      inputRef.current?.focus();
    }
  }, [hasPendingPrompt, showTerminal, selectedId, runningIds]);

  return (
    <div className="app">
      <TitleBar theme={theme} onToggleTheme={toggleTheme} onError={setError} />

      <div className="body">
        <main className="content" ref={contentRef}>
          <div className="toolbar">
            <h2 className="toolbar-title">
              脚本列表
              <span className="toolbar-count">{scripts.length}</span>
            </h2>
            <div className="toolbar-buttons">
              <button className="import-button" onClick={openConfig}>
                打开配置
              </button>
              <button className="import-button" onClick={importScript}>
                导入文件
              </button>
              <button className="import-button" onClick={beginAdd}>
                <PlusIcon size={14} />
                添加
              </button>
            </div>
          </div>

          {error && <p className="error-message">{error}</p>}
          {configError && <p className="error-message">{configError}</p>}
          {configPath && <p className="config-path" title={configPath}>配置文件：{configPath}</p>}

          {showAddForm && (
            <form className="entry-form" onSubmit={saveEntry}>
              <div className="entry-form-heading">
                {editingId ? `编辑 ${scriptName(editingId)}` : "新建条目"}
              </div>
              {editingId && runningIds.includes(editingId) && (
                <p className="entry-form-hint">
                  启动设置在下次运行时生效；停止设置在下次停止此条目时生效。
                </p>
              )}
              <div className="entry-mode">
                <label><input type="radio" name="mode" checked={draft.mode === "file"}
                  onChange={() => setDraft((current) => ({ ...current, mode: "file" }))} /> 文件</label>
                <label><input type="radio" name="mode" checked={draft.mode === "command"}
                  onChange={() => setDraft((current) => ({ ...current, mode: "command" }))} /> 命令</label>
              </div>
              <label className="entry-label">名称
                <input value={draft.name} onChange={(event) => setDraft((current) => ({ ...current, name: event.target.value }))}
                  placeholder={draft.mode === "file" ? "默认使用文件名" : "请输入服务名称"} />
              </label>
              {draft.mode === "file" ? (
                <label className="entry-label">启动文件
                  <div className="entry-path-row">
                    <input value={draft.path} onChange={(event) => setDraft((current) => ({ ...current, path: event.target.value }))}
                      placeholder="C:\\path\\start.cmd" required />
                    <button type="button" onClick={() => pickEntryFile("path")}>浏览</button>
                  </div>
                </label>
              ) : (
                <>
                  <label className="entry-label">命令解释器
                    <select value={draft.shell} onChange={(event) => setDraft((current) => ({ ...current, shell: event.target.value }))}>
                      <option value="cmd">cmd</option><option value="pwsh">pwsh</option><option value="bash">bash</option>
                    </select>
                  </label>
                  <label className="entry-label">启动命令
                    <textarea value={draft.command} onChange={(event) => setDraft((current) => ({ ...current, command: event.target.value }))}
                      placeholder="node server.js" required rows={2} />
                  </label>
                </>
              )}
              <label className="entry-label">工作目录（可选）
                <input value={draft.working_dir} onChange={(event) => setDraft((current) => ({ ...current, working_dir: event.target.value }))}
                  placeholder={draft.mode === "file" ? "默认使用脚本所在目录" : "默认使用管理器所在目录"} />
              </label>
              <label className="entry-label">停止操作（可选）
                <select value={draft.stopMode} onChange={(event) => setDraft((current) => ({ ...current, stopMode: event.target.value }))}>
                  <option value="none">{draft.monitorMode === "tcp" ? "未配置停止操作（停止按钮禁用）" : "结束进程树"}</option>
                  <option value="file">运行停止文件，必要时结束进程树</option>
                  <option value="command">运行停止命令，必要时结束进程树</option>
                </select>
              </label>
              <label className="entry-label">运行状态检测
                <select value={draft.monitorMode} onChange={(event) => setDraft((current) => ({ ...current, monitorMode: event.target.value }))}>
                  <option value="process">跟踪启动进程</option>
                  <option value="tcp">TCP 端口检测（后台服务）</option>
                </select>
              </label>
              {draft.monitorMode === "tcp" && (
                <>
                  <label className="entry-label">检测地址
                    <input value={draft.monitorHost} required placeholder="127.0.0.1"
                      onChange={(event) => setDraft((current) => ({ ...current, monitorHost: event.target.value }))} />
                  </label>
                  <label className="entry-label">检测端口
                    <input type="number" min="1" max="65535" step="1" required value={draft.monitorPort} placeholder="8080"
                      onChange={(event) => setDraft((current) => ({ ...current, monitorPort: event.target.value }))} />
                  </label>
                  <p className="entry-form-hint">自动检测端口，本机状态通常在 2–5 秒内更新。端口可连接表示运行，不代表应用健康。后台服务需要配置停止文件或命令；不支持暂停和标准输入。</p>
                </>
              )}
              {draft.stopMode === "file" && (
                <label className="entry-label">停止文件
                  <div className="entry-path-row">
                    <input value={draft.stopPath} onChange={(event) => setDraft((current) => ({ ...current, stopPath: event.target.value }))}
                      placeholder="C:\\path\\stop.cmd" required />
                    <button type="button" onClick={() => pickEntryFile("stopPath")}>浏览</button>
                  </div>
                </label>
              )}
              {draft.stopMode === "command" && (
                <>
                  <label className="entry-label">停止命令解释器
                    <select value={draft.stopShell} onChange={(event) => setDraft((current) => ({ ...current, stopShell: event.target.value }))}>
                      <option value="cmd">cmd</option><option value="pwsh">pwsh</option><option value="bash">bash</option>
                    </select>
                  </label>
                  <label className="entry-label">停止命令
                    <textarea value={draft.stopCommand} onChange={(event) => setDraft((current) => ({ ...current, stopCommand: event.target.value }))}
                      placeholder="taskkill /PID %SCRIPT_MANAGER_PID% /T /F" required rows={2} />
                  </label>
                </>
              )}
              <div className="entry-form-actions">
                <button type="button" onClick={cancelEntry}>取消</button>
                <button type="submit" className="entry-save">{editingId ? "保存修改" : "添加条目"}</button>
              </div>
            </form>
          )}

          {scripts.length === 0 ? (
            <div className="empty-state">
              <FileCodeIcon size={36} />
              <p>还没有脚本</p>
              <p className="empty-state-hint">
                添加文件或命令，即可开始使用。
              </p>
            </div>
          ) : (
            <ul className="script-list">
              {scripts.map((script) => {
                const runtime = runtimeFor(script);
                const pending = isPendingState(runtime) || busyIds.includes(script.id);
                const isRunning = runningIds.includes(script.id);
                const isPaused = pausedIds.includes(script.id);
                const isSelected = showTerminal && selectedId === script.id;
                const hasStopAction = Boolean(
                  script.stop && (script.stop.path || script.stop.command),
                );

                const statusClass = runtime.status;
                const statusTitle = statusLabel(runtime.status);

                return (
                  <li
                    key={script.id}
                    className={`script-item ${isSelected ? "selected" : ""}`}
                    onClick={() => toggleSelection(script.id)}
                    onContextMenu={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                      setContextMenu({
                        x: Math.min(event.clientX, window.innerWidth - 170),
                        y: Math.min(event.clientY, window.innerHeight - 220),
                        id: script.id,
                      });
                    }}
                  >
                    <span
                      className={`status-dot ${statusClass}`}
                      title={statusTitle}
                    />
                    <div className="script-info">
                      <span className="script-name">{script.name}</span>
                      <span className="script-path" title={script.mode === "command" ? script.command : script.path}>
                        {script.mode === "command" ? `${script.shell}: ${script.command}` : script.path}
                      </span>
                      {script.monitor && <span className={`script-monitor-status ${runtime.status}`} title={runtime.message}>
                        TCP {script.monitor.host}:{script.monitor.port} · {statusTitle}
                        {runtime.status === "error" && runtime.message ? `：${runtime.message}` : ""}
                      </span>}
                    </div>
                    <div className="script-actions">
                      <button
                        className="icon-button edit"
                        onClick={rowAction(beginEdit, script.id)}
                        aria-label={`编辑 ${script.name}`}
                        title="编辑"
                      >
                        <EditIcon size={14} />
                      </button>
                      {isRunning && !isPaused && !script.monitor && (
                        <button
                          className="icon-button pause"
                          onClick={rowAction(pauseScript, script.id)}
                          aria-label={`暂停 ${script.name}`}
                          title="暂停"
                        >
                          <PauseIcon size={14} />
                        </button>
                      )}
                      {isRunning && isPaused && !script.monitor && (
                        <button
                          className="icon-button pause"
                          onClick={rowAction(resumeScript, script.id)}
                          aria-label={`继续 ${script.name}`}
                          title="继续"
                        >
                          <PlayIcon size={14} />
                        </button>
                      )}
                      {!isRunning && (
                        <button
                          className="icon-button run"
                          disabled={pending}
                          onClick={rowAction(startScript, script.id)}
                          aria-label={`运行 ${script.name}`}
                          title="运行"
                        >
                          <PlayIcon size={14} />
                        </button>
                      )}
                      {(isRunning || hasStopAction) && (
                        <button
                          className="icon-button stop"
                          disabled={pending || (Boolean(script.monitor) && !hasStopAction)}
                          onClick={rowAction(stopScript, script.id)}
                          aria-label={`停止 ${script.name}`}
                          title={script.monitor && !hasStopAction ? "请先配置停止操作" : isRunning ? "停止" : "执行停止操作"}
                        >
                          <StopIcon size={14} />
                        </button>
                      )}
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </main>

        {showTerminal && (
          <aside className="terminal">
            <div className="terminal-header">
              <span className="terminal-header-title">
                <TerminalIcon size={13} />
                {selectedId ? scriptName(selectedId) : "全部脚本"}
              </span>
              <button
                className="terminal-clear"
                onClick={() => setLogLines([])}
                aria-label="清空输出"
                title="清空输出"
              >
                <TrashIcon size={13} />
              </button>
            </div>
            <div className="terminal-lines" ref={terminalRef}>
              {visibleLines.length === 0 ? (
                <span className="terminal-line info">
                  {selectedId
                    ? `${scriptName(selectedId)} 暂无输出。`
                    : "暂无输出。"}
                </span>
              ) : (
                visibleLines.map((entry, index) => {
                  const text =
                    entry.stream === "stdin" ? `› ${entry.line}` : entry.line;

                  return (
                    <span
                      key={index}
                      className={`terminal-line ${entry.stream}`}
                    >
                      {selectedId
                        ? text
                        : `[${scriptName(entry.id)}] ${text}`}
                    </span>
                  );
                })
              )}
            </div>
            <div className="terminal-input-row">
              <span className="terminal-input-prompt">›</span>
              <input
                ref={inputRef}
                className="terminal-input"
                type="text"
                value={inputText}
                disabled={!selectedId || !runningIds.includes(selectedId) || Boolean(scripts.find((script) => script.id === selectedId)?.monitor)}
                placeholder={
                  scripts.find((script) => script.id === selectedId)?.monitor
                    ? "端口检测服务不支持标准输入"
                    : !selectedId
                    ? "选择脚本后可发送输入"
                    : !runningIds.includes(selectedId)
                      ? `${scriptName(selectedId)} 未运行`
                      : `向 ${scriptName(selectedId)} 发送输入`
                }
                aria-label="脚本输入"
                onChange={(event) => setInputText(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    sendInput();
                  }
                }}
              />
            </div>
          </aside>
        )}
      </div>

      <footer className="bottom-bar">
        <button
          className={`terminal-toggle ${showTerminal ? "active" : ""}`}
          onClick={() => setShowTerminal((shown) => !shown)}
          aria-label={showTerminal ? "隐藏终端" : "显示终端"}
        >
          <TerminalIcon size={13} />
          终端
        </button>

        <span className="status-summary">
          {scripts.length} 个脚本 · {runningIds.length} 个运行中
        </span>
      </footer>

      {contextMenu &&
        (() => {
          const isRunning = runningIds.includes(contextMenu.id);
          const isPaused = pausedIds.includes(contextMenu.id);
          const contextScript = scripts.find(
            (script) => script.id === contextMenu.id,
          );
          const runtime = runtimeFor(contextScript);
          const pending = isPendingState(runtime) || busyIds.includes(contextMenu.id);
          const hasStopAction = Boolean(
            contextScript?.stop &&
              (contextScript.stop.path || contextScript.stop.command),
          );

          /** Closes the menu, then runs the chosen action. */
          function menuAction(handler) {
            return () => {
              setContextMenu(null);
              handler(contextMenu.id);
            };
          }

          return (
            <div
              className="context-menu"
              style={{ left: contextMenu.x, top: contextMenu.y }}
            >
              {!isRunning && (
                <button
                  className="context-menu-item"
                  onClick={menuAction(startScript)}
                  disabled={pending}
                >
                  <PlayIcon size={13} />
                  运行
                </button>
              )}
              {isRunning && !isPaused && !contextScript?.monitor && (
                <button
                  className="context-menu-item"
                  onClick={menuAction(pauseScript)}
                >
                  <PauseIcon size={13} />
                  暂停
                </button>
              )}
              {isRunning && isPaused && !contextScript?.monitor && (
                <button
                  className="context-menu-item"
                  onClick={menuAction(resumeScript)}
                >
                  <PlayIcon size={13} />
                  继续
                </button>
              )}
              {(isRunning || hasStopAction) && (
                <button
                  className="context-menu-item"
                  onClick={menuAction(stopScript)}
                  disabled={pending || (Boolean(contextScript?.monitor) && !hasStopAction)}
                >
                  <StopIcon size={13} />
                  {isRunning ? "停止" : "执行停止操作"}
                </button>
              )}
              <button
                className="context-menu-item"
                onClick={menuAction(beginEdit)}
              >
                <EditIcon size={13} />
                编辑
              </button>
              <div className="context-menu-separator" />
              <button
                className="context-menu-item danger"
                onClick={menuAction(removeScript)}
                disabled={pending}
              >
                <TrashIcon size={13} />
                移除
              </button>
            </div>
          );
        })()}

      {["North", "South", "East", "West", "NorthEast", "NorthWest", "SouthEast", "SouthWest"].map((direction) => (
        <div
          key={direction}
          className={`window-resize window-resize-${direction}`}
          aria-hidden="true"
          onMouseDown={async (event) => {
            if (event.button !== 0) return;
            event.preventDefault();
            try {
              await getCurrentWindow().startResizeDragging(direction);
            } catch (message) {
              setError(`无法调整窗口大小：${message}`);
            }
          }}
        />
      ))}

      {isDragging && (
        <div className="drop-overlay">
          <PlusIcon size={30} />
          <span>松开鼠标导入脚本</span>
        </div>
      )}
    </div>
  );
}

export default App;
