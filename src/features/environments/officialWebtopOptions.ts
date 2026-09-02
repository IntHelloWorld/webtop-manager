export type OfficialOptionGroup = "base" | "stream" | "features" | "sharing" | "interface" | "hardening";

export interface OfficialOptionDefinition {
  key: string;
  group: OfficialOptionGroup;
  kind: "boolean" | "enum" | "number" | "range" | "text";
  defaultValue: string;
  options?: readonly string[];
  placeholder?: string;
  lockable?: boolean;
}

const booleanOption = (
  key: string,
  group: OfficialOptionGroup,
  defaultValue: "true" | "false",
  lockable = false,
): OfficialOptionDefinition => ({ key, group, kind: "boolean", defaultValue, lockable });

const rangeOption = (
  key: string,
  defaultValue: string,
  options: readonly string[],
): OfficialOptionDefinition => ({ key, group: "stream", kind: "range", defaultValue, options });

export const officialWebtopOptions: readonly OfficialOptionDefinition[] = [
  { key: "DOCKER_MODS", group: "base", kind: "text", defaultValue: "linuxserver/mods:universal-package-install", placeholder: "linuxserver/mods:universal-package-install" },
  { key: "UMASK", group: "base", kind: "enum", defaultValue: "022", options: ["000", "002", "022", "027", "077"] },
  { key: "SELKIES_DESKTOP", group: "base", kind: "boolean", defaultValue: "true" },
  { key: "PELORUS", group: "base", kind: "boolean", defaultValue: "false" },
  { key: "CUSTOM_PORT", group: "base", kind: "number", defaultValue: "3000" },
  { key: "CUSTOM_HTTPS_PORT", group: "base", kind: "number", defaultValue: "3001" },
  { key: "CUSTOM_WS_PORT", group: "base", kind: "number", defaultValue: "8082" },
  { key: "DRI_NODE", group: "base", kind: "text", defaultValue: "/dev/dri/renderD128", placeholder: "/dev/dri/renderD128" },
  { key: "DRINODE", group: "base", kind: "text", defaultValue: "/dev/dri/renderD128", placeholder: "/dev/dri/renderD128" },
  { key: "PIXELFLUX_RECORDING_SOCKET", group: "base", kind: "text", defaultValue: "/defaults/pixelflux_record", placeholder: "/defaults/pixelflux_record" },
  { key: "PIXELFLUX_CU", group: "base", kind: "number", defaultValue: "8084" },
  { key: "SUBFOLDER", group: "base", kind: "text", defaultValue: "/webtop/", placeholder: "/webtop/" },
  { key: "TITLE", group: "base", kind: "text", defaultValue: "Selkies" },
  { key: "DASHBOARD", group: "base", kind: "enum", defaultValue: "selkies-dashboard", options: ["selkies-dashboard", "selkies-dashboard-zinc", "selkies-dashboard-wish"] },
  { key: "FILE_MANAGER_PATH", group: "base", kind: "text", defaultValue: "/config", placeholder: "/config/Downloads" },
  booleanOption("START_DOCKER", "base", "false"),
  booleanOption("DISABLE_IPV6", "base", "false"),
  booleanOption("NO_DECOR", "base", "false"),
  booleanOption("NO_FULL", "base", "false"),
  booleanOption("NO_GAMEPAD", "base", "false"),
  booleanOption("DISABLE_ZINK", "base", "false"),
  booleanOption("DISABLE_DRI3", "base", "false"),
  { key: "MAX_RES", group: "base", kind: "enum", defaultValue: "15360x8640", options: ["1920x1080", "2560x1440", "3840x2160", "7680x4320", "15360x8640"] },
  { key: "WATERMARK_PNG", group: "base", kind: "text", defaultValue: "/usr/share/selkies/www/icon.png", placeholder: "/usr/share/selkies/www/icon.png" },
  { key: "WATERMARK_LOCATION", group: "base", kind: "enum", defaultValue: "1", options: ["1", "2", "3", "4", "5", "6"] },

  { key: "SELKIES_ENCODER", group: "stream", kind: "enum", defaultValue: "x264enc,x264enc-striped,jpeg", options: ["x264enc,x264enc-striped,jpeg", "x264enc", "x264enc-striped", "jpeg"] },
  rangeOption("SELKIES_FRAMERATE", "8-120", ["8-120", "30", "60", "120"]),
  rangeOption("SELKIES_H264_CRF", "5-50", ["5-50", "18", "25", "35"]),
  rangeOption("SELKIES_JPEG_QUALITY", "1-100", ["1-100", "40", "75", "90"]),
  booleanOption("SELKIES_H264_FULLCOLOR", "stream", "false", true),
  booleanOption("SELKIES_H264_STREAMING_MODE", "stream", "false", true),
  booleanOption("SELKIES_FORCE_ALIGNED_RESOLUTION", "stream", "false", true),
  booleanOption("SELKIES_USE_CPU", "stream", "false", true),
  booleanOption("SELKIES_USE_PAINT_OVER_QUALITY", "stream", "true", true),
  rangeOption("SELKIES_PAINT_OVER_JPEG_QUALITY", "1-100", ["1-100", "75", "90", "100"]),
  rangeOption("SELKIES_H264_PAINTOVER_CRF", "5-50", ["5-50", "18", "25", "35"]),
  rangeOption("SELKIES_H264_PAINTOVER_BURST_FRAMES", "1-30", ["1-30", "5", "10", "20"]),
  booleanOption("SELKIES_SECOND_SCREEN", "stream", "true", true),
  { key: "SELKIES_AUDIO_BITRATE", group: "stream", kind: "enum", defaultValue: "320000", options: ["128000", "192000", "256000", "320000"] },
  { key: "SELKIES_SCALING_DPI", group: "stream", kind: "enum", defaultValue: "96", options: ["96", "120", "144", "168", "192", "216", "240", "264", "288"] },
  booleanOption("SELKIES_USE_BROWSER_CURSORS", "stream", "false", true),
  booleanOption("SELKIES_USE_CSS_SCALING", "stream", "false", true),
  { key: "SELKIES_CONTROL_PORT", group: "stream", kind: "number", defaultValue: "8083" },
  { key: "SELKIES_AUDIO_DEVICE_NAME", group: "stream", kind: "text", defaultValue: "output.monitor" },
  { key: "SELKIES_WAYLAND_SOCKET_INDEX", group: "stream", kind: "number", defaultValue: "0" },

  booleanOption("SELKIES_MICROPHONE_ENABLED", "features", "true", true),
  booleanOption("SELKIES_GAMEPAD_ENABLED", "features", "true", true),
  booleanOption("SELKIES_CLIPBOARD_IN_ENABLED", "features", "true", true),
  booleanOption("SELKIES_CLIPBOARD_OUT_ENABLED", "features", "true", true),
  booleanOption("SELKIES_ENABLE_BINARY_CLIPBOARD", "features", "false", true),
  booleanOption("SELKIES_COMMAND_ENABLED", "features", "true", true),
  booleanOption("SELKIES_DEBUG", "features", "false", true),

  booleanOption("SELKIES_ENABLE_SHARING", "sharing", "true", true),
  booleanOption("SELKIES_ENABLE_COLLAB", "sharing", "true", true),
  booleanOption("SELKIES_ENABLE_SHARED", "sharing", "true", true),
  booleanOption("SELKIES_ENABLE_PLAYER2", "sharing", "true", true),
  booleanOption("SELKIES_ENABLE_PLAYER3", "sharing", "true", true),
  booleanOption("SELKIES_ENABLE_PLAYER4", "sharing", "true", true),

  { key: "SELKIES_UI_TITLE", group: "interface", kind: "text", defaultValue: "Selkies" },
  booleanOption("SELKIES_UI_SHOW_LOGO", "interface", "true", true),
  booleanOption("SELKIES_UI_SHOW_SIDEBAR", "interface", "true", true),
  booleanOption("SELKIES_UI_SHOW_CORE_BUTTONS", "interface", "true", true),
  booleanOption("SELKIES_UI_SIDEBAR_SHOW_VIDEO_SETTINGS", "interface", "true", true),
  booleanOption("SELKIES_UI_SIDEBAR_SHOW_SCREEN_SETTINGS", "interface", "true", true),
  booleanOption("SELKIES_UI_SIDEBAR_SHOW_AUDIO_SETTINGS", "interface", "true", true),
  booleanOption("SELKIES_UI_SIDEBAR_SHOW_STATS", "interface", "true", true),
  booleanOption("SELKIES_UI_SIDEBAR_SHOW_CLIPBOARD", "interface", "true", true),
  booleanOption("SELKIES_UI_SIDEBAR_SHOW_FILES", "interface", "true", true),
  booleanOption("SELKIES_UI_SIDEBAR_SHOW_APPS", "interface", "true", true),
  booleanOption("SELKIES_UI_SIDEBAR_SHOW_SHARING", "interface", "true", true),
  booleanOption("SELKIES_UI_SIDEBAR_SHOW_GAMEPADS", "interface", "true", true),
  booleanOption("SELKIES_UI_SIDEBAR_SHOW_FULLSCREEN", "interface", "true", true),
  booleanOption("SELKIES_UI_SIDEBAR_SHOW_GAMING_MODE", "interface", "true", true),
  booleanOption("SELKIES_UI_SIDEBAR_SHOW_TRACKPAD", "interface", "true", true),
  booleanOption("SELKIES_UI_SIDEBAR_SHOW_KEYBOARD_BUTTON", "interface", "true", true),
  booleanOption("SELKIES_UI_SIDEBAR_SHOW_SOFT_BUTTONS", "interface", "true", true),

  booleanOption("HARDEN_DESKTOP", "hardening", "false"),
  booleanOption("HARDEN_OPENBOX", "hardening", "false"),
  booleanOption("DISABLE_OPEN_TOOLS", "hardening", "false"),
  booleanOption("DISABLE_SUDO", "hardening", "false"),
  booleanOption("DISABLE_TERMINALS", "hardening", "false"),
  booleanOption("DISABLE_CLOSE_BUTTON", "hardening", "false"),
  booleanOption("DISABLE_MOUSE_BUTTONS", "hardening", "false"),
  booleanOption("HARDEN_KEYBINDS", "hardening", "false"),
  booleanOption("RESTART_APP", "hardening", "false"),
] as const;

export const officialOptionGroups: readonly OfficialOptionGroup[] = [
  "base",
  "stream",
  "features",
  "sharing",
  "interface",
  "hardening",
];

export function getOfficialOption(key: string) {
  return officialWebtopOptions.find((option) => option.key === key);
}

export function valuesForOption(option: OfficialOptionDefinition): readonly string[] | null {
  if (option.options) return option.options;
  if (option.kind !== "boolean") return null;
  return option.lockable
    ? ["true", "false", "true|locked", "false|locked"]
    : ["true", "false"];
}
