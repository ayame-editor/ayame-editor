// Ayame Editor — non-blocking operation notifications.
//
// Messages are appended instead of replacing one status-bar string, so a
// follow-up operation cannot erase the result that preceded it. Informational
// messages expire independently; errors remain until explicitly dismissed.
import { button, el, iconSvg } from "./dom.js";
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

/// The card already showing exactly this message, if one is still on screen.
function liveNotification(host: HTMLElement, message: string, isError: boolean) {
  for (const card of host.children) {
    if (!(card instanceof HTMLElement) || card.classList.contains("closing")) continue;
    if (card.classList.contains("error") !== isError) continue;
    if (card.querySelector(".notification-message")?.textContent === message) return card;
  }
  return null;
}

function scheduleAutoDismiss(id: number) {
  clearTimer(autoDismissTimers, id);
  autoDismissTimers.set(
    id,
    window.setTimeout(() => dismissNotification(id), NOTIFICATION_DURATION_MS),
  );
}

// `flashCount` is kept as the public name for compatibility with existing
// callers. The message is already localized by the caller.
export function flashCount(msg, kind = "") {
  const message = String(msg || "");
  const host = notificationHost();
  if (!message || !host) return null;

  const isError = kind === "error";
  // A message that is already on screen is the same news, not more of it. An
  // operation that keeps failing — the match count re-requested on every
  // keystroke while the server needs a restart — used to stack one identical
  // card per attempt, and error cards never expire on their own. The existing
  // card stands for all of them; an informational one gets a fresh lifetime so
  // the latest occurrence is still read.
  const live = liveNotification(host, message, isError);
  if (live) {
    const liveId = Number(live.dataset.notificationId);
    if (!isError) scheduleAutoDismiss(liveId);
    return liveId;
  }

  const id = nextNotificationId++;
  const notification = el("div", `notification ${isError ? "error" : "info"}`);
  notification.dataset.notificationId = String(id);

  const text = el("span", "notification-message", message);
  text.setAttribute("role", isError ? "alert" : "status");
  text.setAttribute("aria-live", isError ? "assertive" : "polite");
  text.setAttribute("aria-atomic", "true");

  const dismiss = button("notification-dismiss", "", () => dismissNotification(id));
  dismiss.title = t("notification.dismiss");
  dismiss.setAttribute("aria-label", t("notification.dismiss"));
  dismiss.append(iconSvg("i-close"));

  notification.append(text, dismiss);
  host.append(notification);

  if (!isError) scheduleAutoDismiss(id);
  return id;
}
