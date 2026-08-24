// Ayame Editor — shared overwrite confirmation flow.
import { displayPath } from "./dom.js";
import { isApiErrorCode } from "./api.js";
import { askConfirm } from "./dialogs.js";
import { t } from "./i18n.js";

export function isExistsError(error: unknown) {
  return isApiErrorCode(error, "exists");
}

export async function withOverwriteRetry<T>(
  name: string,
  attempt: (overwrite: boolean) => Promise<T>,
  overwrite = false,
): Promise<T | null> {
  try {
    return await attempt(overwrite);
  } catch (error) {
    if (overwrite || !isExistsError(error)) throw error;
  }

  const confirmed = await askConfirm(
    t("dialog.overwrite.title"),
    t("dialog.overwrite.ask", { name: displayPath(name) }),
    { okLabel: t("dialog.overwrite.ok"), danger: true },
  );
  if (!confirmed) return null;
  return attempt(true);
}
