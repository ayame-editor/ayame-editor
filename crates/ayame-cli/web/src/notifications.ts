// Ayame Editor — non-blocking operation notifications.
//
// Messages are appended instead of replacing one status-bar string, so a
// follow-up operation cannot erase the result that preceded it. Informational
// messages expire independently; errors remain until explicitly dismissed.
import { iconSvg } from "./dom.js";
import { t } from "./i18n.js";

export const NOTIFICATION_DURATION_MS = 6000;
export const NOTIFICATION_EXIT_MS = 160;

let nextNotificationId = 1;
const autoDismissTimers = new Map<number, number>();
const exitTimers = new Map<number, number>();

function notificationHost() {
  return document.getElementById("notifications");
}

function notificationElement(id: number) {
  return notificationHost()?.querySelector<HTMLElement>(`[data-notification-id="${id}"]`) ?? null;
}

function clearTimer(timers: Map<number, number>, id: number) {
  const timer = timers.get(id);
  if (timer === undefined) return;
  window.clearTimeout(timer);
  timers.delete(id);
}

function removeNotification(id: number) {
  clearTimer(autoDismissTimers, id);
  clearTimer(exitTimers, id);
  notificationElement(id)?.remove();
}

export function dismissNotification(id: number) {
  clearTimer(autoDismissTimers, id);
  const notification = notificationElement(id);
  if (!notification || notification.classList.contains("closing")) return;
  notification.classList.add("closing");
  exitTimers.set(
    id,
    window.setTimeout(() => removeNotification(id), NOTIFICATION_EXIT_MS),
  );
}

export function clearNotifications() {
  for (const timer of autoDismissTimers.values()) window.clearTimeout(timer);
  for (const timer of exitTimers.values()) window.clearTimeout(timer);
  autoDismissTimers.clear();
  exitTimers.clear();
  notificationHost()?.replaceChildren();
}

// `flashCount` is kept as the public name for compatibility with existing
// callers. The message is already localized by the caller.
export function flashCount(msg, kind = "") {
  const message = String(msg || "");
  const host = notificationHost();
  if (!message || !host) return null;

  const id = nextNotificationId++;
  const isError = kind === "error";
  const notification = document.createElement("div");
  notification.className = `notification ${isError ? "error" : "info"}`;
  notification.dataset.notificationId = String(id);

  const text = document.createElement("span");
  text.className = "notification-message";
  text.textContent = message;
  text.setAttribute("role", isError ? "alert" : "status");
  text.setAttribute("aria-live", isError ? "assertive" : "polite");
  text.setAttribute("aria-atomic", "true");

  const dismiss = document.createElement("button");
  dismiss.type = "button";
  dismiss.className = "notification-dismiss";
  dismiss.title = t("notification.dismiss");
  dismiss.setAttribute("aria-label", t("notification.dismiss"));
  dismiss.append(iconSvg("i-close"));
  dismiss.addEventListener("click", () => dismissNotification(id));

  notification.append(text, dismiss);
  host.append(notification);

  if (!isError) {
    autoDismissTimers.set(
      id,
      window.setTimeout(() => dismissNotification(id), NOTIFICATION_DURATION_MS),
    );
  }
  return id;
}
