// Shell-free external analysis actions (#248).
import { state } from "./state.js";
import { t } from "./i18n.js";
import { apiPost } from "./api.js";
import {
  askConfirm,
  askForm,
  hideLoading,
  newOperationId,
  showLoading,
  showMessage,
} from "./dialogs.js";
import { flashCount } from "./notifications.js";
import { rectRange, selRange } from "./selection.js";
import { newUntitled } from "./workspace.js";
import { pasteText } from "./edits.js";

const CONFIG_KEY = "ayame.externalAction.v1";

type InputMode = "file" | "snapshot" | "selection_stdin" | "selection_file";
type OutputMode = "panel" | "new_tab" | "file";

type ActionConfig = {
  name: string;
  executable: string;
  arguments: string[];
  input: InputMode;
  output: OutputMode;
  timeout_ms: number;
  max_output_bytes: number;
  working_directory: string | null;
};

type ActionResult = {
  name: string;
  success: boolean;
  exit_code: number | null;
  timed_out: boolean;
  canceled: boolean;
  duration_ms: number;
  stdout: string;
  stderr: string;
  stdout_truncated: boolean;
  stderr_truncated: boolean;
  output_path: string | null;
};

const DEFAULT_CONFIG: ActionConfig = {
  name: "Log analyzer",
  executable: "",
  arguments: ["{snapshot_file}"],
  input: "snapshot",
  output: "panel",
  timeout_ms: 30_000,
  max_output_bytes: 1024 * 1024,
  working_directory: "{dir}",
};

function loadConfig(): ActionConfig {
  try {
    const value = JSON.parse(localStorage.getItem(CONFIG_KEY) || "null");
    return value && typeof value === "object" ? { ...DEFAULT_CONFIG, ...value } : DEFAULT_CONFIG;
  } catch {
    return DEFAULT_CONFIG;
  }
}

function saveConfig(config: ActionConfig) {
  try {
    localStorage.setItem(CONFIG_KEY, JSON.stringify(config));
  } catch {
    // Private browsing may reject persistence; running the confirmed action is
    // still safe and useful for this session.
  }
}

function selectionRequest() {
  const rect = rectRange();
  if (rect) {
    return { rect: true, l0: rect.l0, c0: rect.c0, l1: rect.l1, c1: rect.c1 };
  }
  const range = selRange();
  if (!range || (range.start.line === range.end.line && range.start.col === range.end.col)) {
    return null;
  }
  return {
    rect: false,
    l0: range.start.line,
    c0: range.start.col,
    l1: range.end.line,
    c1: range.end.col,
  };
}

function parseArguments(value: string): string[] {
  const parsed = JSON.parse(value);
  if (!Array.isArray(parsed) || !parsed.every((entry) => typeof entry === "string")) {
    throw new Error(t("externalAction.argumentsArrayError"));
  }
  return parsed;
}

function resultText(result: ActionResult) {
  const exit = result.canceled
    ? t("externalAction.canceled")
    : result.timed_out
      ? t("externalAction.timedOut")
      : result.exit_code == null
        ? t("externalAction.noExitCode")
        : String(result.exit_code);
  const trunc = (cut: boolean) => (cut ? `\n[${t("externalAction.outputTruncated")}]` : "");
  return [
    `${t("externalAction.exitCode")}: ${exit}`,
    `${t("externalAction.duration")}: ${result.duration_ms} ms`,
    result.output_path ? `${t("externalAction.outputFile")}: ${result.output_path}` : "",
    `\n--- stdout ---\n${result.stdout || t("externalAction.empty")}${trunc(result.stdout_truncated)}`,
    `\n--- stderr ---\n${result.stderr || t("externalAction.empty")}${trunc(result.stderr_truncated)}`,
  ]
    .filter(Boolean)
    .join("\n");
}

export async function configureAndRunExternalAction() {
  if (!state.doc.stat?.open) return;
  const previous = loadConfig();
  const values = await askForm<{
    configJson: string;
    name: string;
    executable: string;
    arguments: string;
    input: string;
    output: string;
    outputPath: string;
    workingDirectory: string;
    timeout: string;
    maxOutput: string;
  }>(t("externalAction.title"), [
    {
      id: "configJson",
      type: "text",
      label: t("externalAction.configJson"),
      value: "",
      placeholder: t("externalAction.configJsonPlaceholder"),
      title: t("externalAction.configJsonHint", { json: JSON.stringify(previous) }),
    },
    { id: "name", type: "text", label: t("externalAction.name"), value: previous.name },
    {
      id: "executable",
      type: "path",
      label: t("externalAction.executable"),
      value: previous.executable,
    },
    {
      id: "arguments",
      type: "text",
      label: t("externalAction.arguments"),
      value: JSON.stringify(previous.arguments),
      title: t("externalAction.argumentsHint"),
    },
    {
      id: "input",
      type: "select",
      label: t("externalAction.input"),
      value: previous.input,
      options: [
        ["file", t("externalAction.inputFile")],
        ["snapshot", t("externalAction.inputSnapshot")],
        ["selection_stdin", t("externalAction.inputSelectionStdin")],
        ["selection_file", t("externalAction.inputSelectionFile")],
      ],
    },
    {
      id: "output",
      type: "select",
      label: t("externalAction.output"),
      value: previous.output,
      options: [
        ["panel", t("externalAction.outputPanel")],
        ["new_tab", t("externalAction.outputTab")],
        ["file", t("externalAction.outputFile")],
      ],
    },
    { id: "outputPath", type: "path", label: t("externalAction.outputPath"), value: "" },
    {
      id: "workingDirectory",
      type: "path",
      label: t("externalAction.workingDirectory"),
      value: previous.working_directory || "",
    },
    {
      id: "timeout",
      type: "text",
      label: t("externalAction.timeout"),
      value: String(previous.timeout_ms / 1000),
    },
    {
      id: "maxOutput",
      type: "text",
      label: t("externalAction.maxOutput"),
      value: String(previous.max_output_bytes / (1024 * 1024)),
    },
    { type: "hint", label: t("externalAction.placeholderHint") },
  ]);
  if (!values) return;

  let config: ActionConfig;
  try {
    const timeout = Number(values.timeout);
    const maxOutput = Number(values.maxOutput);
    const formConfig: ActionConfig = {
      name: values.name.trim(),
      executable: values.executable.trim(),
      arguments: parseArguments(values.arguments),
      input: values.input as InputMode,
      output: values.output as OutputMode,
      timeout_ms: Math.round(timeout * 1000),
      max_output_bytes: Math.round(maxOutput * 1024 * 1024),
      working_directory: values.workingDirectory.trim() || null,
    };
    const imported = values.configJson.trim() ? JSON.parse(values.configJson) : null;
    if (imported && (typeof imported !== "object" || Array.isArray(imported))) {
      throw new Error(t("externalAction.invalidConfig"));
    }
    config = imported ? { ...formConfig, ...imported } : formConfig;
    config.arguments = Array.isArray(config.arguments)
      ? parseArguments(JSON.stringify(config.arguments))
      : parseArguments("");
    if (
      typeof config.name !== "string" ||
      !config.name ||
      typeof config.executable !== "string" ||
      !config.executable ||
      !["file", "snapshot", "selection_stdin", "selection_file"].includes(config.input) ||
      !["panel", "new_tab", "file"].includes(config.output) ||
      !Number.isFinite(config.timeout_ms) ||
      !Number.isFinite(config.max_output_bytes) ||
      (config.working_directory != null && typeof config.working_directory !== "string")
    ) {
      throw new Error(t("externalAction.invalidConfig"));
    }
  } catch (error) {
    await showMessage(t("externalAction.invalidConfig"), (error as Error).message);
    return;
  }
  const selection = selectionRequest();
  if (config.input.startsWith("selection_") && !selection) {
    flashCount(t("externalAction.selectionRequired"), "error");
    return;
  }
  if (config.output === "file" && !values.outputPath.trim()) {
    flashCount(t("externalAction.outputPathRequired"), "error");
    return;
  }

  saveConfig(config);
  const approved = await askConfirm(
    t("externalAction.confirmTitle"),
    [
      `${t("externalAction.executable")}: ${config.executable}`,
      `${t("externalAction.arguments")}: ${JSON.stringify(config.arguments, null, 2)}`,
      `${t("externalAction.workingDirectory")}: ${config.working_directory || "-"}`,
      "",
      t("externalAction.confirmWarning"),
    ].join("\n"),
    { okLabel: t("common.run") },
  );
  if (!approved) return;

  const opId = newOperationId("external-action");
  showLoading(t("externalAction.running", { name: config.name }), { opId, cancel: true });
  try {
    const result = await apiPost<ActionResult>("/api/actions/run", {
      config,
      approved: true,
      op_id: opId,
      line: state.caret.position.line + 1,
      column: state.caret.position.col + 1,
      selection,
      output_path: values.outputPath.trim() || null,
      overwrite: false,
    });
    hideLoading();
    if (config.output === "new_tab" && result.stdout) {
      await newUntitled();
      pasteText(result.stdout);
    }
    await showMessage(config.name, resultText(result));
  } catch (error) {
    hideLoading();
    await showMessage(t("externalAction.failed"), (error as Error).message);
  }
}
