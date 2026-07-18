import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  clearNotifications,
  flashCount,
  NOTIFICATION_DURATION_MS,
  NOTIFICATION_EXIT_MS,
} from "../src/notifications.js";
import { state } from "../src/state.js";

let originalLanguage;

beforeEach(() => {
  vi.useFakeTimers();
  originalLanguage = state.settings.language;
  state.settings.language = "en";
  document.body.innerHTML = '<div id="notifications"></div>';
});

afterEach(() => {
  clearNotifications();
  vi.clearAllTimers();
  vi.useRealTimers();
  state.settings.language = originalLanguage;
});

describe("operation notification queue (#177)", () => {
  it("keeps consecutive messages for their own full lifetime", async () => {
    const savedId = flashCount("Saved");
    await vi.advanceTimersByTimeAsync(1000);
    const searchId = flashCount("3 matches");

    const messages = () =>
      [...document.querySelectorAll(".notification-message")].map((node) => node.textContent);
    expect(messages()).toEqual(["Saved", "3 matches"]);

    await vi.advanceTimersByTimeAsync(NOTIFICATION_DURATION_MS - 1000);
    expect(
      document.querySelector(`[data-notification-id="${savedId}"]`)?.classList,
    ).toContain("closing");
    expect(
      document.querySelector(`[data-notification-id="${searchId}"]`)?.classList,
    ).not.toContain("closing");

    await vi.advanceTimersByTimeAsync(NOTIFICATION_EXIT_MS);
    expect(messages()).toEqual(["3 matches"]);
  });

  it("announces normal messages politely and lets them be dismissed early", async () => {
    const id = flashCount("Saved");
    const notification = document.querySelector(`[data-notification-id="${id}"]`);
    const message = notification?.querySelector(".notification-message");
    const dismiss = notification?.querySelector<HTMLButtonElement>(".notification-dismiss");

    expect(message?.getAttribute("role")).toBe("status");
    expect(message?.getAttribute("aria-live")).toBe("polite");
    expect(message?.getAttribute("aria-atomic")).toBe("true");
    expect(dismiss?.getAttribute("aria-label")).toBe("Dismiss notification");

    dismiss?.click();
    await vi.advanceTimersByTimeAsync(NOTIFICATION_EXIT_MS);
    expect(document.querySelector(`[data-notification-id="${id}"]`)).toBeNull();
  });

  it("keeps errors until manual dismiss and announces them assertively", async () => {
    const id = flashCount("Save failed", "error");
    const notification = document.querySelector(`[data-notification-id="${id}"]`);
    const message = notification?.querySelector(".notification-message");

    expect(notification?.classList).toContain("error");
    expect(message?.getAttribute("role")).toBe("alert");
    expect(message?.getAttribute("aria-live")).toBe("assertive");

    await vi.advanceTimersByTimeAsync(NOTIFICATION_DURATION_MS * 3);
    expect(document.querySelector(`[data-notification-id="${id}"]`)).not.toBeNull();

    notification?.querySelector<HTMLButtonElement>(".notification-dismiss")?.click();
    await vi.advanceTimersByTimeAsync(NOTIFICATION_EXIT_MS);
    expect(document.querySelector(`[data-notification-id="${id}"]`)).toBeNull();
  });
});
