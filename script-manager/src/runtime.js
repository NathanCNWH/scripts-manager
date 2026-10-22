export function isRunningState(state) {
  return ["running", "paused", "stopping"].includes(state.status);
}

export function isPendingState(state) {
  return ["checking", "starting", "stopping"].includes(state.status);
}

export function statusLabel(status) {
  return {
    checking: "检测中",
    starting: "启动中",
    running: "运行中",
    paused: "已暂停",
    stopping: "停止中",
    stopped: "已停止",
    error: "检测异常",
  }[status] || "检测异常";
}
