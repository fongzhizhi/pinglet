/**
 * @ai-generated deepseek-v4-flash
 * 设置面板：主题、字号、面板透明度、通知、系统行为、鼠标穿透、快捷键与关于信息。
 */
import {
  getSettings,
  PANEL_TRANSPARENCY_MAX,
  PANEL_TRANSPARENCY_MIN,
  PANEL_TRANSPARENCY_STEP,
  setAutoStartSetting,
  setClickThroughSetting,
  setCloseToTraySetting,
  setDoNotDisturbSetting,
  setFontSize,
  setPanelEffect,
  setReminder,
  setShortcutSetting,
  setSystemNotificationsSetting,
  setTheme,
  setTransparencySetting,
  subscribeSettings,
  type FontSize,
  type ReminderKey,
  type ThemeMode,
} from "./settings";
import {
  IS_TAURI,
  getAutoStart,
  quitApp,
  setAutoStart,
  setCloseToTray,
  setGlobalShortcut,
  toMessage,
} from "./system";

export type SettingsPanelHandle = {
  syncSystemState: () => Promise<void>;
};

type PreviewableReminderKey = Exclude<ReminderKey, "none">;

type SettingsPanelOptions = {
  onNotice: (message: string) => void;
  onPreviewReminder: (key: PreviewableReminderKey) => void;
  onResetSettings: () => Promise<void>;
  onClickThroughChange: (enabled: boolean) => void;
  onClose: () => void;
};

type SegmentedOption<T extends string> = {
  value: T;
  label: string;
};

type ReminderOption = {
  key: ReminderKey;
  label: string;
  description: string;
};

const THEME_OPTIONS: readonly SegmentedOption<ThemeMode>[] = [
  { value: "dark", label: "暗色" },
  { value: "light", label: "亮色" },
  { value: "auto", label: "自动" },
];

const FONT_SIZE_OPTIONS: readonly SegmentedOption<FontSize>[] = [
  { value: "small", label: "小" },
  { value: "standard", label: "标准" },
  { value: "large", label: "大" },
];

const REMINDER_OPTIONS: readonly ReminderOption[] = [
  { key: "breathing", label: "呼吸灯", description: "异常或完成时边框发光" },
  { key: "sound", label: "音效", description: "发出轻微提示音" },
  {
    key: "shake",
    label: "窗口抖动",
    description: "异常时抖动三次，完成时抖动一次",
  },
  { key: "none", label: "无", description: "关闭全部提醒" },
];

const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "Meta"]);
const KEY_ALIASES: Readonly<Record<string, string>> = {
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  PageUp: "PageUp",
  PageDown: "PageDown",
  Home: "Home",
  End: "End",
  Insert: "Insert",
  Delete: "Delete",
  Enter: "Enter",
  Tab: "Tab",
  Spacebar: "Space",
};
const FUNCTION_KEY_PATTERN = /^F\d{1,2}$/;
const STATUS_RESET_DELAY_MS = 4000;

function createSection(title: string): HTMLElement {
  const section = document.createElement("section");
  section.className = "settings-section";

  const heading = document.createElement("h2");
  heading.className = "settings-section-title";
  heading.textContent = title;

  section.append(heading);
  return section;
}

function createSettingRow(labelText: string): {
  row: HTMLDivElement;
  label: HTMLSpanElement;
} {
  const row = document.createElement("div");
  row.className = "setting-row";

  const label = document.createElement("span");
  label.className = "setting-label";
  label.textContent = labelText;

  row.append(label);
  return { row, label };
}

function createSwitch(
  checked: boolean,
  disabled: boolean,
  onChange: (checked: boolean) => void,
): HTMLInputElement {
  const wrapper = document.createElement("label");
  wrapper.className = "switch";

  const input = document.createElement("input");
  input.type = "checkbox";
  input.checked = checked;
  input.disabled = disabled;

  const track = document.createElement("span");
  track.className = "switch-track";

  input.addEventListener("change", () => {
    onChange(input.checked);
  });

  wrapper.append(input, track);
  return input;
}

function createSwitchRow(
  labelText: string,
  checked: boolean,
  disabled: boolean,
  onChange: (checked: boolean) => void,
): { row: HTMLDivElement; input: HTMLInputElement } {
  const { row } = createSettingRow(labelText);
  const input = createSwitch(checked, disabled, onChange);
  row.append(input.parentElement as HTMLLabelElement);
  return { row, input };
}

function createSegmentedControl<T extends string>(
  ariaLabel: string,
  options: readonly SegmentedOption<T>[],
  selected: T,
  onChange: (value: T) => void,
): HTMLDivElement {
  const group = document.createElement("div");
  group.className = "segmented";
  group.setAttribute("role", "group");
  group.setAttribute("aria-label", ariaLabel);

  for (const option of options) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "segmented-btn";
    button.textContent = option.label;
    button.dataset.value = option.value;
    button.setAttribute("aria-pressed", String(option.value === selected));
    button.addEventListener("click", () => {
      onChange(option.value);
    });
    group.append(button);
  }

  return group;
}

function syncSegmentedControl<T extends string>(group: HTMLElement, selected: T): void {
  for (const button of group.querySelectorAll<HTMLButtonElement>("button[data-value]")) {
    button.setAttribute("aria-pressed", String(button.dataset.value === selected));
  }
}

function createReminderRow(
  option: ReminderOption,
  checked: boolean,
  onPreview: (key: PreviewableReminderKey) => void,
): { row: HTMLLabelElement; input: HTMLInputElement } {
  const row = document.createElement("label");
  row.className = "check-row";
  row.title = option.description;

  const input = document.createElement("input");
  input.type = "checkbox";
  input.checked = checked;
  input.dataset.reminderKey = option.key;

  const text = document.createElement("span");
  text.textContent = option.label;

  input.addEventListener("change", () => {
    setReminder(option.key, input.checked);
    // 勾选后立即演示对应效果，方便选择（“无”没有可演示的效果）
    if (input.checked && option.key !== "none") {
      onPreview(option.key);
    }
  });

  row.append(input, text);
  return { row, input };
}

function normalizeKey(key: string): string | null {
  if (key === " ") {
    return "Space";
  }
  if (key.length === 1) {
    return key.toUpperCase();
  }
  if (FUNCTION_KEY_PATTERN.test(key)) {
    return key.toUpperCase();
  }
  return KEY_ALIASES[key] ?? null;
}

function acceleratorFromEvent(event: KeyboardEvent): string | null {
  if (MODIFIER_KEYS.has(event.key)) {
    return null;
  }

  const modifiers: string[] = [];
  if (event.ctrlKey) {
    modifiers.push("Ctrl");
  }
  if (event.altKey) {
    modifiers.push("Alt");
  }
  if (event.shiftKey) {
    modifiers.push("Shift");
  }
  if (event.metaKey) {
    modifiers.push("Super");
  }
  if (modifiers.length === 0) {
    return null;
  }

  const key = normalizeKey(event.key);
  if (key === null) {
    return null;
  }
  return [...modifiers, key].join("+");
}

function formatTransparency(transparency: number): string {
  return `${Math.round(transparency * 100)}%`;
}

export function mountSettingsPanel(
  container: HTMLElement,
  options: SettingsPanelOptions,
): SettingsPanelHandle {
  const initial = getSettings();
  const fragment = document.createDocumentFragment();

  // ============ 顶部：标题与返回 ============
  const header = document.createElement("div");
  header.className = "settings-header";

  const headerTitle = document.createElement("span");
  headerTitle.className = "settings-header-title";
  headerTitle.textContent = "设置";

  const backButton = document.createElement("button");
  backButton.type = "button";
  backButton.className = "settings-back-btn";
  backButton.textContent = "← 返回";
  backButton.setAttribute("aria-label", "隐藏设置面板");
  backButton.addEventListener("click", () => {
    options.onClose();
  });

  header.append(headerTitle, backButton);
  fragment.append(header);

  // ============ 外观与提醒 ============
  const appearanceSection = createSection("外观与提醒");

  const themeRow = createSettingRow("主题");
  const themeControl = createSegmentedControl("主题", THEME_OPTIONS, initial.theme, (theme) => {
    setTheme(theme);
  });
  themeRow.row.append(themeControl);
  appearanceSection.append(themeRow.row);

  const fontSizeRow = createSettingRow("字号");
  const fontSizeControl = createSegmentedControl(
    "字号",
    FONT_SIZE_OPTIONS,
    initial.fontSize,
    (fontSize) => {
      setFontSize(fontSize);
    },
  );
  fontSizeRow.row.append(fontSizeControl);
  appearanceSection.append(fontSizeRow.row);

  const transparencyRow = createSettingRow("透明度");
  const transparencyControl = document.createElement("div");
  transparencyControl.className = "transparency-control";

  const transparencySlider = document.createElement("input");
  transparencySlider.type = "range";
  transparencySlider.className = "transparency-slider";
  transparencySlider.min = String(Math.round(PANEL_TRANSPARENCY_MIN * 100));
  transparencySlider.max = String(Math.round(PANEL_TRANSPARENCY_MAX * 100));
  transparencySlider.step = String(Math.round(PANEL_TRANSPARENCY_STEP * 100));
  transparencySlider.value = String(Math.round(initial.transparency * 100));
  transparencySlider.setAttribute("aria-label", "面板透明度");

  const transparencyValue = document.createElement("span");
  transparencyValue.className = "transparency-value";
  transparencyValue.textContent = formatTransparency(initial.transparency);

  transparencySlider.addEventListener("input", () => {
    setTransparencySetting(Number(transparencySlider.value) / 100);
  });

  transparencyControl.append(transparencySlider, transparencyValue);
  transparencyRow.row.append(transparencyControl);
  appearanceSection.append(transparencyRow.row);

  const reminderRow = createSettingRow("提醒方式");
  reminderRow.row.title = "窗口可见时的窗口内提醒，与通知消息相互独立";
  const reminderGroup = document.createElement("div");
  reminderGroup.className = "reminder-group";
  const reminderInputs: HTMLInputElement[] = [];
  for (const option of REMINDER_OPTIONS) {
    const { row, input } = createReminderRow(
      option,
      initial.reminders[option.key],
      options.onPreviewReminder,
    );
    reminderInputs.push(input);
    reminderGroup.append(row);
  }
  reminderRow.row.append(reminderGroup);
  appearanceSection.append(reminderRow.row);

  const alertPulseRow = createSwitchRow(
    "异常行呼吸灯",
    initial.panelEffects.alertPulse,
    false,
    (enabled) => {
      setPanelEffect("alertPulse", enabled);
    },
  );
  alertPulseRow.row.title =
    "待处理 / 已中断会话行持续呼吸：背景呼吸 + 状态灯扩散光环；与「提醒方式」的一次性提醒相互独立";
  appearanceSection.append(alertPulseRow.row);

  const workingFlowRow = createSwitchRow(
    "进行中行流光",
    initial.panelEffects.workingFlow,
    false,
    (enabled) => {
      setPanelEffect("workingFlow", enabled);
    },
  );
  workingFlowRow.row.title = "进行中会话行底部循环流动的光带，表示任务仍在执行";
  appearanceSection.append(workingFlowRow.row);

  const systemNotificationsRow = createSwitchRow(
    "通知消息",
    initial.systemNotifications,
    !IS_TAURI,
    (enabled) => {
      setSystemNotificationsSetting(enabled);
    },
  );
  systemNotificationsRow.row.title =
    "仅在窗口隐藏或最小化且出现新异常时，在主屏右下角弹出应用内通知卡片，3 秒后自动隐藏；与提醒方式相互独立";
  appearanceSection.append(systemNotificationsRow.row);

  const doNotDisturbRow = createSwitchRow("免打扰", initial.doNotDisturb, false, (enabled) => {
    setDoNotDisturbSetting(enabled);
  });
  // NOTE: 这里只控制 Pinglet 自己的提醒链路；Windows 系统「请勿打扰」是另一层开关，
  // 且 Windows 11 22621 上没有可靠的 API 能读取它，因此不做自动探测。
  doNotDisturbRow.row.title =
    "暂停应用内的全部提醒与托盘闪烁，保留托盘告警色和计数；与 Windows 系统「请勿打扰」相互独立";
  appearanceSection.append(doNotDisturbRow.row);

  fragment.append(appearanceSection);

  // ============ 系统与行为 ============
  const systemSection = createSection("系统与行为");

  const autoStartRow = createSwitchRow("开机自启", initial.autoStart, !IS_TAURI, (enabled) => {
    void applyAutoStart(enabled);
  });
  systemSection.append(autoStartRow.row);

  const closeToTrayRow = createSwitchRow(
    "点击关闭时最小化到托盘",
    initial.closeToTray,
    false,
    (enabled) => {
      setCloseToTraySetting(enabled);
      if (!IS_TAURI) {
        return;
      }
      void setCloseToTray(enabled).catch((error: unknown) => {
        setCloseToTraySetting(!enabled);
        options.onNotice(`设置关闭行为失败: ${toMessage(error)}`);
      });
    },
  );
  systemSection.append(closeToTrayRow.row);

  const clickThroughRow = createSwitchRow(
    "鼠标穿透",
    initial.clickThrough,
    !IS_TAURI,
    (enabled) => {
      if (!IS_TAURI) {
        clickThroughRow.input.checked = getSettings().clickThrough;
        showStatus("浏览器预览模式：鼠标穿透不可用", true);
        return;
      }
      // 实际生效时机由外部调度：开启需等设置面板关闭，避免面板立刻无法点击
      setClickThroughSetting(enabled);
      options.onClickThroughChange(enabled);
    },
  );
  systemSection.append(clickThroughRow.row);

  const shortcutRow = createSettingRow("全局快捷键");
  const shortcutInput = document.createElement("input");
  shortcutInput.type = "text";
  shortcutInput.className = "shortcut-input";
  shortcutInput.readOnly = true;
  shortcutInput.disabled = !IS_TAURI;
  shortcutInput.placeholder = "按下组合键";
  shortcutInput.value = initial.shortcut;
  shortcutInput.setAttribute("aria-label", "全局快捷键");
  shortcutRow.row.append(shortcutInput);
  systemSection.append(shortcutRow.row);

  const status = document.createElement("p");
  status.className = "settings-status";
  status.setAttribute("role", "status");
  systemSection.append(status);

  fragment.append(systemSection);

  // ============ 关于 ============
  const aboutSection = createSection("关于");
  const version = document.createElement("p");
  version.className = "about-line";
  version.textContent = `Pinglet v${__APP_VERSION__}`;
  const author = document.createElement("p");
  author.className = "about-line";
  author.textContent = "Designed & Built by fongzhizhi";
  aboutSection.append(version, author);
  fragment.append(aboutSection);

  // ============ 底部 ============
  const actions = document.createElement("div");
  actions.className = "settings-actions";

  const resetButton = document.createElement("button");
  resetButton.type = "button";
  resetButton.className = "reset-btn";
  resetButton.textContent = "恢复默认";
  resetButton.addEventListener("click", () => {
    resetButton.disabled = true;
    void options
      .onResetSettings()
      .then(() => {
        showStatus("已恢复默认设置", false);
      })
      .catch((error: unknown) => {
        showStatus(`恢复默认设置失败: ${toMessage(error)}`, true);
      })
      .finally(() => {
        resetButton.disabled = false;
      });
  });

  const quitButton = document.createElement("button");
  quitButton.type = "button";
  quitButton.className = "quit-btn";
  quitButton.textContent = "退出应用";
  quitButton.addEventListener("click", () => {
    if (!IS_TAURI) {
      options.onNotice("浏览器预览模式：退出应用不可用");
      return;
    }
    void quitApp().catch((error: unknown) => {
      options.onNotice(`退出应用失败: ${toMessage(error)}`);
    });
  });
  actions.append(resetButton, quitButton);
  fragment.append(actions);

  container.replaceChildren(fragment);

  let statusTimer: number | undefined;

  function showStatus(message: string, isError: boolean): void {
    status.textContent = message;
    status.classList.toggle("is-error", isError);
    if (statusTimer !== undefined) {
      window.clearTimeout(statusTimer);
    }
    statusTimer = window.setTimeout(() => {
      status.textContent = "";
      status.classList.remove("is-error");
    }, STATUS_RESET_DELAY_MS);
  }

  async function applyShortcut(shortcut: string): Promise<void> {
    shortcutInput.disabled = true;
    try {
      await setGlobalShortcut(shortcut === "" ? null : shortcut);
      setShortcutSetting(shortcut);
      showStatus(shortcut === "" ? "已清除全局快捷键" : `已注册 ${shortcut}`, false);
    } catch (error) {
      // 注册失败时 Rust 会恢复旧快捷键，输入框同步回已生效的值
      shortcutInput.value = getSettings().shortcut;
      showStatus(`快捷键不可用: ${toMessage(error)}`, true);
    } finally {
      shortcutInput.disabled = !IS_TAURI;
    }
  }

  async function applyAutoStart(enabled: boolean): Promise<void> {
    if (!IS_TAURI) {
      autoStartRow.input.checked = getSettings().autoStart;
      options.onNotice("浏览器预览模式：开机自启不可用");
      return;
    }

    autoStartRow.input.disabled = true;
    try {
      await setAutoStart(enabled);
      setAutoStartSetting(enabled);
      showStatus(enabled ? "已开启开机自启" : "已关闭开机自启", false);
    } catch (error) {
      autoStartRow.input.checked = !enabled;
      showStatus(`开机自启设置失败: ${toMessage(error)}`, true);
      options.onNotice(`设置开机自启失败: ${toMessage(error)}`);
    } finally {
      autoStartRow.input.disabled = !IS_TAURI;
    }
  }

  shortcutInput.addEventListener("keydown", (event) => {
    if (!IS_TAURI) {
      return;
    }

    if (
      event.key === "Backspace" &&
      !event.ctrlKey &&
      !event.altKey &&
      !event.shiftKey &&
      !event.metaKey
    ) {
      event.preventDefault();
      event.stopPropagation();
      void applyShortcut("");
      return;
    }

    const shortcut = acceleratorFromEvent(event);
    if (shortcut === null) {
      if (!MODIFIER_KEYS.has(event.key) && event.key !== "Escape") {
        event.preventDefault();
        showStatus("请至少包含 Ctrl / Alt / Shift / Win 之一", true);
      }
      return;
    }

    event.preventDefault();
    event.stopPropagation();
    void applyShortcut(shortcut);
  });

  function syncControls(): void {
    const current = getSettings();

    syncSegmentedControl(themeControl, current.theme);
    syncSegmentedControl(fontSizeControl, current.fontSize);
    transparencySlider.value = String(Math.round(current.transparency * 100));
    transparencyValue.textContent = formatTransparency(current.transparency);

    for (const input of reminderInputs) {
      const key = input.dataset.reminderKey as ReminderKey;
      input.checked = current.reminders[key];
    }

    alertPulseRow.input.checked = current.panelEffects.alertPulse;
    workingFlowRow.input.checked = current.panelEffects.workingFlow;

    autoStartRow.input.checked = current.autoStart;
    closeToTrayRow.input.checked = current.closeToTray;
    clickThroughRow.input.checked = current.clickThrough;
    shortcutInput.value = current.shortcut;
    systemNotificationsRow.input.checked = current.systemNotifications;
    doNotDisturbRow.input.checked = current.doNotDisturb;
  }

  async function syncSystemState(): Promise<void> {
    if (!IS_TAURI) {
      return;
    }
    try {
      const enabled = await getAutoStart();
      setAutoStartSetting(enabled);
    } catch (error) {
      showStatus(`读取开机自启状态失败: ${toMessage(error)}`, true);
    }
  }

  subscribeSettings(syncControls);
  syncControls();

  return { syncSystemState };
}
