<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

type ClipboardKind = "text" | "link" | "files" | "image";

type ClipboardItem = {
  id: string;
  createdAt: number;
  kind: ClipboardKind;
  text?: string;
  url?: string;
  paths?: string[];
  path?: string;
  width?: number;
  height?: number;
};

type ThemeName =
  | "system"
  | "light"
  | "dark"
  | "tokyonight"
  | "everforest"
  | "ayu"
  | "catppuccin"
  | "catppuccin-macchiato"
  | "gruvbox"
  | "kanagawa"
  | "nord"
  | "matrix"
  | "one-dark";

type HistorySettings = {
  maxItems: number;
  autoPaste: boolean;
  capturePaused: boolean;
  theme: ThemeName;
};

type CaptureStatus = {
  lastCaptureAt?: number;
  lastError?: string | null;
};

type StoredState = {
  items: ClipboardItem[];
  settings: HistorySettings;
  status?: CaptureStatus;
};

const appWindow = getCurrentWindow();
const items = ref<ClipboardItem[]>([]);
const search = ref("");
const isOpen = ref(false);
const isMac = ref(false);
const searchInput = ref<HTMLInputElement | null>(null);
const settingsButton = ref<HTMLButtonElement | null>(null);
const settingsPanel = ref<HTMLDivElement | null>(null);
const historyItemRefs = ref<HTMLElement[]>([]);
const activeKeys = reactive({
  meta: false,
  shift: false,
  v: false,
  up: false,
  down: false,
  left: false,
  right: false,
  enter: false,
  backspace: false,
});
const settings = reactive<HistorySettings>({
  maxItems: 50,
  autoPaste: false,
  capturePaused: false,
  theme: "light",
});
const status = reactive<CaptureStatus>({
  lastCaptureAt: undefined,
  lastError: undefined,
});
const isSettingsOpen = ref(false);
const selectedIndex = ref(0);
const pageIndex = ref(0);
const systemPrefersDark = ref(false);
let prefersDarkQuery: MediaQueryList | null = null;
let isClosing = false;

const themeOptions: Array<{ label: string; value: ThemeName }> = [
  { label: "System", value: "system" },
  { label: "Light", value: "light" },
  { label: "Dark", value: "dark" },
  { label: "Tokyonight", value: "tokyonight" },
  { label: "Everforest", value: "everforest" },
  { label: "Ayu", value: "ayu" },
  { label: "Catppuccin", value: "catppuccin" },
  { label: "Catppuccin Macchiato", value: "catppuccin-macchiato" },
  { label: "Gruvbox", value: "gruvbox" },
  { label: "Kanagawa", value: "kanagawa" },
  { label: "Nord", value: "nord" },
  { label: "Matrix", value: "matrix" },
  { label: "One Dark", value: "one-dark" },
];

const maxVisibleItems = 5;
const pageSize = computed(() => Math.min(settings.maxItems, maxVisibleItems));
const primaryModifier = computed(() => (isMac.value ? "⌘" : "Ctrl"));
const resolvedTheme = computed(() => {
  if (settings.theme === "system") {
    return systemPrefersDark.value ? "dark" : "light";
  }
  return settings.theme;
});
const themeClass = computed(() => `theme-${resolvedTheme.value}`);
const unlistenHistory = ref<(() => void) | null>(null);
const unlistenOpen = ref<(() => void) | null>(null);
const unlistenClose = ref<(() => void) | null>(null);
const unlistenFocus = ref<(() => void) | null>(null);
const filteredItems = computed(() => {
  const query = search.value.trim().toLowerCase();
  if (!query) {
    return items.value;
  }

  return items.value.filter((item) => {
    const content = buildSearchText(item);
    return content.includes(query);
  });
});

const totalPages = computed(() =>
  Math.max(1, Math.ceil(filteredItems.value.length / pageSize.value))
);
const pageStart = computed(() => pageIndex.value * pageSize.value);
const visibleItems = computed(() =>
  filteredItems.value.slice(pageStart.value, pageStart.value + pageSize.value)
);

const statusMessage = computed(() => {
  if (settings.capturePaused) {
    return "Capture paused";
  }
  if (status.lastError) {
    return status.lastError;
  }
  return "";
});

const isApplyingState = ref(false);
const hasThemeOverride = ref(false);

function applyState(payload: StoredState) {
  isApplyingState.value = true;
  const currentTheme = settings.theme;
  items.value = payload.items;
  Object.assign(settings, payload.settings);
  Object.assign(status, payload.status ?? {});
  if (hasThemeOverride.value && settings.theme !== currentTheme) {
    settings.theme = currentTheme;
  }
  const themeNeedsSync = hasThemeOverride.value && payload.settings.theme !== currentTheme;
  Promise.resolve().then(() => {
    isApplyingState.value = false;
    if (themeNeedsSync) {
      void updateSettings();
    }
  });
}

async function refreshHistory() {
  const payload = await invoke<StoredState>("get_history");
  applyState(payload);
}

async function openOverlay() {
  if (isOpen.value) {
    await appWindow.setFocus();
    return;
  }
  isClosing = false;
  isOpen.value = true;
  await appWindow.show();
  await appWindow.setAlwaysOnTop(true);
  await appWindow.center();
  await appWindow.setFocus();
  await refreshHistory();
  isSettingsOpen.value = false;
  selectedIndex.value = 0;
  pageIndex.value = 0;
  await nextTick();
  searchInput.value?.focus();
}

async function closeOverlay() {
  if (!isOpen.value || isClosing) {
    return;
  }
  isClosing = true;
  isOpen.value = false;
  search.value = "";
  isSettingsOpen.value = false;
  selectedIndex.value = 0;
  pageIndex.value = 0;
  try {
    await appWindow.setAlwaysOnTop(false);
    await appWindow.hide();
  } finally {
    isClosing = false;
  }
}

async function selectItem(item: ClipboardItem) {
  const currentTheme = settings.theme;
  try {
    const payload = await invoke<StoredState>("select_item", { id: item.id });
    applyState(payload);
    if (settings.theme !== currentTheme) {
      settings.theme = currentTheme;
      void updateSettings();
    }
  } finally {
    await closeOverlay();
  }
}

async function updateSettings() {
  const payload = await invoke<StoredState>("update_settings", {
    max_items: settings.maxItems,
    auto_paste: settings.autoPaste,
    theme: settings.theme,
  });
  applyState(payload);
}

async function toggleCapture() {
  const payload = await invoke<StoredState>("toggle_capture");
  applyState(payload);
}

async function clearHistory() {
  const currentTheme = settings.theme;
  const payload = await invoke<StoredState>("clear_history");
  applyState(payload);
  if (settings.theme !== currentTheme) {
    settings.theme = currentTheme;
    void updateSettings();
  }
}

function toggleSettings() {
  isSettingsOpen.value = !isSettingsOpen.value;
}

function handleDocumentClick(event: MouseEvent) {
  if (!isSettingsOpen.value) {
    return;
  }
  const target = event.target as Node;
  if (settingsPanel.value?.contains(target) || settingsButton.value?.contains(target)) {
    return;
  }
  isSettingsOpen.value = false;
}

function moveSelection(delta: number) {
  if (visibleItems.value.length === 0) {
    selectedIndex.value = 0;
    return;
  }
  selectedIndex.value = Math.min(
    Math.max(selectedIndex.value + delta, 0),
    visibleItems.value.length - 1
  );
}

function changePage(delta: number) {
  const nextPage = Math.min(Math.max(pageIndex.value + delta, 0), totalPages.value - 1);
  if (nextPage !== pageIndex.value) {
    pageIndex.value = nextPage;
    selectedIndex.value = 0;
  }
}

function selectActiveItem() {
  const item = visibleItems.value[selectedIndex.value];
  if (item) {
    void selectItem(item);
  }
}

function highlightItem(index: number) {
  selectedIndex.value = index;
}

function clearSearch() {
  if (!search.value) {
    return;
  }
  search.value = "";
  pageIndex.value = 0;
  selectedIndex.value = 0;
  void nextTick(() => {
    searchInput.value?.focus();
  });
}

function handleKeydown(event: KeyboardEvent) {
  if (!isOpen.value) {
    return;
  }

  if (event.key === "Escape") {
    event.preventDefault();
    if (isSettingsOpen.value) {
      isSettingsOpen.value = false;
      return;
    }
    const isSearchFocused = document.activeElement === searchInput.value;
    if (isSearchFocused || search.value.trim().length > 0) {
      search.value = "";
      pageIndex.value = 0;
      selectedIndex.value = 0;
      searchInput.value?.blur();
      return;
    }
    void closeOverlay();
    return;
  }

  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "w") {
    event.preventDefault();
    void closeOverlay();
    return;
  }

  if ((event.metaKey || event.ctrlKey) && event.shiftKey && event.key === "Backspace") {
    event.preventDefault();
    activeKeys.backspace = true;
    void clearHistory();
    return;
  }

  if (event.key === "Meta" || event.key === "Control") {
    activeKeys.meta = true;
  }

  if (event.key === "Shift") {
    activeKeys.shift = true;
  }

  if (event.key === "Backspace") {
    activeKeys.backspace = true;
  }

  if (event.key.toLowerCase() === "v") {
    activeKeys.v = true;
  }

  if (event.key === "ArrowDown") {
    event.preventDefault();
    activeKeys.down = true;
    moveSelection(1);
    return;
  }

  if (event.key === "ArrowUp") {
    event.preventDefault();
    activeKeys.up = true;
    moveSelection(-1);
    return;
  }

  if (event.key === "ArrowRight") {
    event.preventDefault();
    activeKeys.right = true;
    changePage(1);
    return;
  }

  if (event.key === "ArrowLeft") {
    event.preventDefault();
    activeKeys.left = true;
    changePage(-1);
    return;
  }

  if (event.key === "Enter" || event.key === "NumpadEnter") {
    event.preventDefault();
    activeKeys.enter = true;
    selectActiveItem();
    return;
  }

  if (/^[1-9]$/.test(event.key) && search.value.trim().length === 0) {
    const index = Number(event.key) - 1;
    if (index >= pageSize.value) {
      return;
    }
    if (visibleItems.value[index]) {
      event.preventDefault();
      highlightItem(index);
    }
  }
}

function handleKeyup(event: KeyboardEvent) {
  if (event.key === "Meta" || event.key === "Control") {
    activeKeys.meta = false;
  }

  if (event.key === "Shift") {
    activeKeys.shift = false;
  }

  if (event.key.toLowerCase() === "v") {
    activeKeys.v = false;
  }

  if (event.key === "ArrowUp") {
    activeKeys.up = false;
  }

  if (event.key === "ArrowDown") {
    activeKeys.down = false;
  }

  if (event.key === "ArrowLeft") {
    activeKeys.left = false;
  }

  if (event.key === "ArrowRight") {
    activeKeys.right = false;
  }

  if (event.key === "Enter" || event.key === "NumpadEnter") {
    activeKeys.enter = false;
  }

  if (event.key === "Backspace") {
    activeKeys.backspace = false;
  }
}

watch(search, () => {
  pageIndex.value = 0;
  if (visibleItems.value.length > 0) {
    selectedIndex.value = 0;
  }
});

watch(
  () => settings.theme,
  () => {
    if (isApplyingState.value) {
      return;
    }
    hasThemeOverride.value = true;
    void updateSettings();
  }
);

watch(totalPages, (value) => {
  if (pageIndex.value > value - 1) {
    pageIndex.value = Math.max(0, value - 1);
  }
});

watch(visibleItems, (nextItems) => {
  if (nextItems.length === 0) {
    selectedIndex.value = 0;
    return;
  }
  if (selectedIndex.value >= nextItems.length) {
    selectedIndex.value = nextItems.length - 1;
  }
});

watch(selectedIndex, (value) => {
  if (value < 0) {
    return;
  }
  void nextTick(() => {
    historyItemRefs.value[value]?.scrollIntoView({ block: "nearest" });
  });
});

function formatTimeAgo(timestamp: number) {
  const diffMs = Date.now() - timestamp;
  const seconds = Math.round(diffMs / 1000);
  const rtf = new Intl.RelativeTimeFormat("en", { numeric: "auto" });

  if (Math.abs(seconds) < 60) {
    return rtf.format(-seconds, "second");
  }

  const minutes = Math.round(seconds / 60);
  if (Math.abs(minutes) < 60) {
    return rtf.format(-minutes, "minute");
  }

  const hours = Math.round(minutes / 60);
  if (Math.abs(hours) < 24) {
    return rtf.format(-hours, "hour");
  }

  const days = Math.round(hours / 24);
  if (Math.abs(days) < 7) {
    return rtf.format(-days, "day");
  }

  const weeks = Math.round(days / 7);
  if (Math.abs(weeks) < 5) {
    return rtf.format(-weeks, "week");
  }

  const months = Math.round(days / 30);
  if (Math.abs(months) < 12) {
    return rtf.format(-months, "month");
  }

  const years = Math.round(days / 365);
  return rtf.format(-years, "year");
}

function buildSearchText(item: ClipboardItem) {
  const base = item.kind === "text" ? item.text ?? "" : "";
  const url = item.kind === "link" ? item.url ?? "" : "";
  const paths = item.kind === "files" ? item.paths?.join(" ") ?? "" : "";
  const image = item.kind === "image" ? item.path ?? "" : "";
  return `${base} ${url} ${paths} ${image}`.toLowerCase();
}

function itemTitle(item: ClipboardItem) {
  if (item.kind === "text") {
    return truncate(item.text ?? "");
  }

  if (item.kind === "link") {
    return truncate(item.url ?? "");
  }

  if (item.kind === "files") {
    if (!item.paths || item.paths.length === 0) {
      return "Untitled";
    }
    if (item.paths.length === 1) {
      return fileName(item.paths[0]);
    }
    return `${fileName(item.paths[0])} +${item.paths.length - 1}`;
  }

  if (item.kind === "image") {
    const dimensions = item.width && item.height ? `${item.width}×${item.height}` : "";
    return dimensions || "Image";
  }

  return "Clipboard";
}

function itemSubtitle(item: ClipboardItem) {
  if (item.kind === "text") {
    return "";
  }

  if (item.kind === "link") {
    return "";
  }

  if (item.kind === "files") {
    if (!item.paths || item.paths.length === 0) {
      return "";
    }
    if (item.paths.length === 1) {
      return "";
    }
    return item.paths.slice(1, 3).map(fileName).join(" · ");
  }

  if (item.kind === "image") {
    return "";
  }

  return "";
}

function itemBadge(item: ClipboardItem) {
  switch (item.kind) {
    case "text":
      return "Text";
    case "link":
      return "Link";
    case "files":
      return "Files";
    case "image":
      return "Image";
    default:
      return "Item";
  }
}

function fileName(path: string) {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

function truncate(value: string, length = 90) {
  if (value.length <= length) {
    return value;
  }
  return `${value.slice(0, length)}…`;
}

function imageSrc(item: ClipboardItem) {
  if (!item.path) {
    return "";
  }
  return convertFileSrc(item.path);
}

function detectMac() {
  if (typeof navigator === "undefined") {
    return false;
  }
  return /mac/i.test(navigator.platform) || /mac/i.test(navigator.userAgent);
}

function updateSystemTheme(event?: MediaQueryList | MediaQueryListEvent) {
  if (event && "matches" in event) {
    systemPrefersDark.value = event.matches;
    return;
  }
  if (prefersDarkQuery) {
    systemPrefersDark.value = prefersDarkQuery.matches;
  }
}

onMounted(async () => {
  isMac.value = detectMac();
  if (typeof window !== "undefined" && "matchMedia" in window) {
    prefersDarkQuery = window.matchMedia("(prefers-color-scheme: dark)");
    updateSystemTheme(prefersDarkQuery);
    prefersDarkQuery.addEventListener("change", updateSystemTheme);
  }
  await refreshHistory();

  unlistenHistory.value = await listen<StoredState>("history-updated", (event) => {
    applyState(event.payload);
  });

  unlistenOpen.value = await listen("open-overlay", () => {
    void openOverlay();
  });

  unlistenClose.value = await listen("close-overlay", () => {
    void closeOverlay();
  });

  unlistenFocus.value = await appWindow.onFocusChanged(({ payload }) => {
    if (!payload) {
      void closeOverlay();
    }
  });

  window.addEventListener("keydown", handleKeydown);
  window.addEventListener("keyup", handleKeyup);
  window.addEventListener("mousedown", handleDocumentClick);

  if (import.meta.env.DEV) {
    await openOverlay();
  } else {
    await closeOverlay();
  }
});

onUnmounted(() => {
  window.removeEventListener("keydown", handleKeydown);
  window.removeEventListener("keyup", handleKeyup);
  window.removeEventListener("mousedown", handleDocumentClick);
  if (prefersDarkQuery) {
    prefersDarkQuery.removeEventListener("change", updateSystemTheme);
    prefersDarkQuery = null;
  }
  unlistenHistory.value?.();
  unlistenOpen.value?.();
  unlistenClose.value?.();
  unlistenFocus.value?.();
});
</script>

<template>
  <div class="backdrop" v-if="isOpen" :class="themeClass" @click="closeOverlay" />
  <div class="app" :class="[themeClass, { 'is-open': isOpen }]">
    <header class="header">
      <div class="header-row">
        <div class="search">
          <span class="search-icon" aria-hidden="true">
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <circle cx="11" cy="11" r="7" stroke="currentColor" stroke-width="2" fill="none" />
              <line
                x1="16.65"
                y1="16.65"
                x2="21"
                y2="21"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
              />
            </svg>
          </span>
          <input
            ref="searchInput"
            v-model="search"
            type="text"
            placeholder="Search clipboard history"
            @keydown.enter.stop.prevent="selectActiveItem"
          />
          <button
            v-if="search.length > 0"
            class="search-clear"
            type="button"
            aria-label="Clear search"
            @mousedown.prevent
            @click="clearSearch"
          >
            <svg viewBox="0 0 12 12" aria-hidden="true">
              <line
                x1="3"
                y1="3"
                x2="9"
                y2="9"
                stroke="currentColor"
                stroke-width="1.6"
                stroke-linecap="round"
              />
              <line
                x1="9"
                y1="3"
                x2="3"
                y2="9"
                stroke="currentColor"
                stroke-width="1.6"
                stroke-linecap="round"
              />
            </svg>
          </button>
        </div>
        <button
          ref="settingsButton"
          class="icon-button settings-button"
          type="button"
          @click="toggleSettings"
          aria-label="Open settings"
          :aria-expanded="isSettingsOpen"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="12" cy="12" r="3" fill="none" stroke="currentColor" stroke-width="1.8" />
            <path
              d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"
              fill="none"
              stroke="currentColor"
              stroke-width="1.6"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
        </button>
      </div>
      <div v-if="isSettingsOpen" ref="settingsPanel" class="settings-popover">
        <div class="settings-title">Settings</div>
        <div class="settings-group">
          <div class="setting">
            <span>Auto paste</span>
            <label class="toggle">
              <input type="checkbox" v-model="settings.autoPaste" @change="updateSettings" />
              <span class="toggle-slider" />
            </label>
          </div>
          <div class="setting">
            <span>Max history</span>
            <input
              type="number"
              min="1"
              max="99"
              v-model.number="settings.maxItems"
              @change="updateSettings"
            />
          </div>
          <div class="setting">
            <span>Theme</span>
            <select v-model="settings.theme" class="settings-select">
              <option v-for="theme in themeOptions" :key="theme.value" :value="theme.value">
                {{ theme.label }}
              </option>
            </select>
          </div>
        </div>
        <div class="settings-actions">
          <button class="ghost" type="button" @click="toggleCapture">
            {{ settings.capturePaused ? "Resume capture" : "Pause capture" }}
          </button>
        </div>
      </div>
    </header>
    <p v-if="statusMessage" class="status" :class="{ error: status.lastError }">
      {{ statusMessage }}
    </p>

    <section class="history">
      <div v-if="visibleItems.length === 0" class="empty">
        <p>No clipboard history yet.</p>
        <span>Copy something to start building your stack.</span>
      </div>
      <button
        v-for="(item, index) in visibleItems"
        :key="item.id"
        type="button"
        class="history-item"
        :class="{ active: index === selectedIndex }"
        ref="historyItemRefs"
        @click="highlightItem(index)"
        @keydown.enter.stop.prevent="selectItem(item)"
      >
        <div class="history-index">
          <span class="mini-keycap">{{ index + 1 }}</span>
        </div>
        <div class="history-content">
          <div class="history-title">{{ itemTitle(item) }}</div>
          <div v-if="itemSubtitle(item)" class="history-subtitle">
            {{ itemSubtitle(item) }}
          </div>
          <div class="history-badge">{{ itemBadge(item) }}</div>
        </div>
        <div class="history-meta">
          <div class="history-time">{{ formatTimeAgo(item.createdAt) }}</div>
          <img
            v-if="item.kind === 'image'"
            :src="imageSrc(item)"
            class="history-thumb"
            alt="Clipboard image preview"
          />
        </div>
      </button>
      <div v-if="totalPages > 1" class="page-indicator page-indicator--bottom" aria-label="History pages">
        <span
          v-for="page in totalPages"
          :key="page"
          class="page-dot"
          :class="{ active: page - 1 === pageIndex }"
        />
      </div>
    </section>

    <footer class="footer">
      <div class="shortcut-block">
        <div class="keycaps">
          <span class="keycap" :class="{ pressed: activeKeys.meta }">
            {{ primaryModifier }}
          </span>
          <span class="keycap-plus">+</span>
          <span class="keycap" :class="{ pressed: activeKeys.shift }">Shift</span>
          <span class="keycap-plus">+</span>
          <span class="keycap" :class="{ pressed: activeKeys.v }">V</span>
        </div>
        <span class="shortcut-label">Show history</span>
      </div>
      <div class="shortcut-block">
        <div class="keycaps">
          <span class="keycap" :class="{ pressed: activeKeys.meta }">
            {{ primaryModifier }}
          </span>
          <span class="keycap-plus">+</span>
          <span class="keycap" :class="{ pressed: activeKeys.shift }">Shift</span>
          <span class="keycap-plus">+</span>
          <span class="keycap keycap--wide" :class="{ pressed: activeKeys.backspace }">
            Backspace
          </span>
        </div>
        <span class="shortcut-label">Clear history</span>
      </div>
      <div class="shortcut-block">
        <div class="keycaps">
          <span class="keycap" :class="{ pressed: activeKeys.left }">←</span>
          <span class="keycap" :class="{ pressed: activeKeys.up }">↑</span>
          <span class="keycap" :class="{ pressed: activeKeys.down }">↓</span>
          <span class="keycap" :class="{ pressed: activeKeys.right }">→</span>
          <span class="keycap keycap--wide" :class="{ pressed: activeKeys.enter }">Enter</span>
        </div>
        <span class="shortcut-label">Navigate & select</span>
      </div>
    </footer>
  </div>
</template>

<style scoped>
.backdrop {
  position: fixed;
  inset: 0;
  transition: background 0.2s ease;
}

.backdrop.theme-light {
  background: transparent;
}

.backdrop.theme-dark,
.backdrop.theme-tokyonight,
.backdrop.theme-everforest,
.backdrop.theme-ayu,
.backdrop.theme-catppuccin,
.backdrop.theme-catppuccin-macchiato,
.backdrop.theme-gruvbox,
.backdrop.theme-kanagawa,
.backdrop.theme-nord,
.backdrop.theme-matrix,
.backdrop.theme-one-dark {
  background: transparent;
}

.app {
  --app-bg: rgba(248, 244, 236, 0.96);
  --app-surface: rgba(255, 255, 255, 0.65);
  --app-surface-strong: rgba(255, 255, 255, 0.9);
  --app-border: rgba(155, 139, 114, 0.28);
  --app-text: #2f2a21;
  --app-muted: rgba(47, 42, 33, 0.6);
  --app-muted-strong: rgba(47, 42, 33, 0.75);
  --app-shadow: 0 30px 80px rgba(80, 64, 40, 0.2);
  --accent: #d6a45b;
  --keycap-bg: linear-gradient(180deg, #fff6ea, #e6d3bc);
  --keycap-border: rgba(139, 113, 82, 0.5);
  --keycap-shadow: #c7b196;
  --keycap-text: #3d3024;
  --toggle-bg: rgba(176, 153, 118, 0.35);
  --toggle-active: rgba(222, 170, 92, 0.9);
  --toggle-thumb: #fdf7ed;
  --history-item-height: 60px;
  --history-gap: 6px;
  position: relative;
  z-index: 1;
  height: 100vh;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 18px;
  box-sizing: border-box;
  overflow: hidden;
  background: var(--app-bg);
  color: var(--app-text);
  border-radius: 26px;
  border: 1px solid var(--app-border);
  box-shadow: var(--app-shadow);
  backdrop-filter: blur(28px);
}

.app.theme-dark {
  --app-bg: rgba(20, 20, 24, 0.9);
  --app-surface: rgba(255, 255, 255, 0.08);
  --app-surface-strong: rgba(255, 255, 255, 0.14);
  --app-border: rgba(255, 255, 255, 0.12);
  --app-text: #f6f4ef;
  --app-muted: rgba(255, 255, 255, 0.55);
  --app-muted-strong: rgba(255, 255, 255, 0.7);
  --app-shadow: 0 30px 90px rgba(0, 0, 0, 0.55);
  --accent: #f4c56b;
  --keycap-bg: linear-gradient(160deg, #f7f3eb, #d7cebf);
  --keycap-border: rgba(193, 183, 167, 0.9);
  --keycap-shadow: #b1a795;
  --keycap-text: #3c372f;
  --toggle-bg: rgba(255, 255, 255, 0.2);
  --toggle-active: rgba(249, 187, 96, 0.75);
  --toggle-thumb: #f7f3eb;
}

.app.theme-tokyonight {
  --app-bg: rgba(26, 27, 38, 0.94);
  --app-surface: rgba(36, 40, 59, 0.72);
  --app-surface-strong: rgba(36, 40, 59, 0.9);
  --app-border: rgba(65, 72, 104, 0.6);
  --app-text: #c0caf5;
  --app-muted: rgba(169, 177, 214, 0.65);
  --app-muted-strong: rgba(169, 177, 214, 0.85);
  --app-shadow: 0 30px 90px rgba(5, 8, 20, 0.6);
  --accent: #7aa2f7;
  --keycap-bg: linear-gradient(160deg, #d5ddf4, #98a9d6);
  --keycap-border: rgba(132, 148, 189, 0.9);
  --keycap-shadow: #7787b8;
  --keycap-text: #2a2f45;
  --toggle-bg: rgba(122, 162, 247, 0.25);
  --toggle-active: rgba(122, 162, 247, 0.75);
  --toggle-thumb: #e7ecfb;
}

.app.theme-everforest {
  --app-bg: rgba(43, 51, 57, 0.94);
  --app-surface: rgba(60, 68, 74, 0.75);
  --app-surface-strong: rgba(60, 68, 74, 0.92);
  --app-border: rgba(97, 111, 104, 0.6);
  --app-text: #d3c6aa;
  --app-muted: rgba(157, 169, 160, 0.7);
  --app-muted-strong: rgba(157, 169, 160, 0.9);
  --app-shadow: 0 30px 90px rgba(12, 16, 18, 0.6);
  --accent: #a7c080;
  --keycap-bg: linear-gradient(160deg, #e1d5bc, #a4b695);
  --keycap-border: rgba(158, 170, 142, 0.9);
  --keycap-shadow: #8fa17d;
  --keycap-text: #2f352f;
  --toggle-bg: rgba(167, 192, 128, 0.25);
  --toggle-active: rgba(167, 192, 128, 0.75);
  --toggle-thumb: #efe8d7;
}

.app.theme-ayu {
  --app-bg: rgba(15, 20, 25, 0.94);
  --app-surface: rgba(31, 36, 48, 0.8);
  --app-surface-strong: rgba(31, 36, 48, 0.92);
  --app-border: rgba(60, 65, 76, 0.6);
  --app-text: #e6e1cf;
  --app-muted: rgba(184, 179, 155, 0.7);
  --app-muted-strong: rgba(184, 179, 155, 0.9);
  --app-shadow: 0 30px 90px rgba(6, 10, 12, 0.65);
  --accent: #ffb454;
  --keycap-bg: linear-gradient(160deg, #f2e7c8, #c9b188);
  --keycap-border: rgba(215, 190, 137, 0.9);
  --keycap-shadow: #b09264;
  --keycap-text: #3f3426;
  --toggle-bg: rgba(255, 180, 84, 0.25);
  --toggle-active: rgba(255, 180, 84, 0.8);
  --toggle-thumb: #f7f1e2;
}

.app.theme-catppuccin {
  --app-bg: rgba(30, 30, 46, 0.95);
  --app-surface: rgba(42, 43, 61, 0.78);
  --app-surface-strong: rgba(42, 43, 61, 0.92);
  --app-border: rgba(69, 71, 90, 0.7);
  --app-text: #cdd6f4;
  --app-muted: rgba(166, 173, 200, 0.7);
  --app-muted-strong: rgba(166, 173, 200, 0.9);
  --app-shadow: 0 30px 90px rgba(10, 9, 20, 0.6);
  --accent: #89b4fa;
  --keycap-bg: linear-gradient(160deg, #d7deef, #9aa9d8);
  --keycap-border: rgba(152, 169, 216, 0.9);
  --keycap-shadow: #7f92c8;
  --keycap-text: #2f3244;
  --toggle-bg: rgba(137, 180, 250, 0.25);
  --toggle-active: rgba(137, 180, 250, 0.8);
  --toggle-thumb: #eef0fb;
}

.app.theme-catppuccin-macchiato {
  --app-bg: rgba(36, 39, 58, 0.95);
  --app-surface: rgba(48, 51, 71, 0.78);
  --app-surface-strong: rgba(48, 51, 71, 0.92);
  --app-border: rgba(73, 77, 100, 0.7);
  --app-text: #cad3f5;
  --app-muted: rgba(165, 173, 203, 0.7);
  --app-muted-strong: rgba(165, 173, 203, 0.9);
  --app-shadow: 0 30px 90px rgba(12, 12, 24, 0.6);
  --accent: #8aadf4;
  --keycap-bg: linear-gradient(160deg, #d7dcef, #98a7d7);
  --keycap-border: rgba(150, 167, 215, 0.9);
  --keycap-shadow: #8093c6;
  --keycap-text: #30354a;
  --toggle-bg: rgba(138, 173, 244, 0.25);
  --toggle-active: rgba(138, 173, 244, 0.8);
  --toggle-thumb: #eef1fb;
}

.app.theme-gruvbox {
  --app-bg: rgba(40, 40, 40, 0.95);
  --app-surface: rgba(60, 56, 54, 0.82);
  --app-surface-strong: rgba(60, 56, 54, 0.94);
  --app-border: rgba(80, 73, 69, 0.7);
  --app-text: #ebdbb2;
  --app-muted: rgba(168, 153, 132, 0.7);
  --app-muted-strong: rgba(168, 153, 132, 0.9);
  --app-shadow: 0 30px 90px rgba(14, 10, 8, 0.65);
  --accent: #fabd2f;
  --keycap-bg: linear-gradient(160deg, #f2e2b9, #c6a96c);
  --keycap-border: rgba(197, 169, 108, 0.9);
  --keycap-shadow: #a68d57;
  --keycap-text: #3a2f1f;
  --toggle-bg: rgba(250, 189, 47, 0.25);
  --toggle-active: rgba(250, 189, 47, 0.8);
  --toggle-thumb: #f8f1d5;
}

.app.theme-kanagawa {
  --app-bg: rgba(31, 31, 40, 0.95);
  --app-surface: rgba(42, 42, 55, 0.78);
  --app-surface-strong: rgba(42, 42, 55, 0.92);
  --app-border: rgba(84, 84, 109, 0.7);
  --app-text: #dcd7ba;
  --app-muted: rgba(161, 152, 133, 0.7);
  --app-muted-strong: rgba(161, 152, 133, 0.9);
  --app-shadow: 0 30px 90px rgba(10, 10, 18, 0.6);
  --accent: #7e9cd8;
  --keycap-bg: linear-gradient(160deg, #e2ddc6, #a0aabf);
  --keycap-border: rgba(146, 160, 199, 0.9);
  --keycap-shadow: #7f8fb5;
  --keycap-text: #2f3347;
  --toggle-bg: rgba(126, 156, 216, 0.25);
  --toggle-active: rgba(126, 156, 216, 0.8);
  --toggle-thumb: #eef1f9;
}

.app.theme-nord {
  --app-bg: rgba(46, 52, 64, 0.95);
  --app-surface: rgba(59, 66, 82, 0.8);
  --app-surface-strong: rgba(59, 66, 82, 0.92);
  --app-border: rgba(76, 86, 106, 0.7);
  --app-text: #eceff4;
  --app-muted: rgba(216, 222, 233, 0.7);
  --app-muted-strong: rgba(216, 222, 233, 0.9);
  --app-shadow: 0 30px 90px rgba(10, 12, 16, 0.6);
  --accent: #88c0d0;
  --keycap-bg: linear-gradient(160deg, #e7eef6, #b8c3d3);
  --keycap-border: rgba(170, 187, 205, 0.9);
  --keycap-shadow: #93a4b9;
  --keycap-text: #2b3648;
  --toggle-bg: rgba(136, 192, 208, 0.25);
  --toggle-active: rgba(136, 192, 208, 0.8);
  --toggle-thumb: #f0f4fa;
}

.app.theme-matrix {
  --app-bg: rgba(9, 14, 11, 0.95);
  --app-surface: rgba(18, 26, 20, 0.8);
  --app-surface-strong: rgba(18, 26, 20, 0.92);
  --app-border: rgba(32, 56, 42, 0.7);
  --app-text: #b8ffcf;
  --app-muted: rgba(111, 220, 145, 0.7);
  --app-muted-strong: rgba(111, 220, 145, 0.9);
  --app-shadow: 0 30px 90px rgba(2, 6, 4, 0.65);
  --accent: #00ff7a;
  --keycap-bg: linear-gradient(160deg, #d3ffe3, #6deaa0);
  --keycap-border: rgba(95, 211, 146, 0.9);
  --keycap-shadow: #4dc281;
  --keycap-text: #0c2a17;
  --toggle-bg: rgba(0, 255, 122, 0.25);
  --toggle-active: rgba(0, 255, 122, 0.8);
  --toggle-thumb: #ebfff4;
}

.app.theme-one-dark {
  --app-bg: rgba(40, 44, 52, 0.95);
  --app-surface: rgba(49, 54, 64, 0.8);
  --app-surface-strong: rgba(49, 54, 64, 0.92);
  --app-border: rgba(62, 68, 81, 0.7);
  --app-text: #abb2bf;
  --app-muted: rgba(171, 178, 191, 0.65);
  --app-muted-strong: rgba(171, 178, 191, 0.85);
  --app-shadow: 0 30px 90px rgba(12, 14, 18, 0.6);
  --accent: #61afef;
  --keycap-bg: linear-gradient(160deg, #d0d5dd, #94a2b3);
  --keycap-border: rgba(148, 162, 179, 0.9);
  --keycap-shadow: #7f8da0;
  --keycap-text: #2b313c;
  --toggle-bg: rgba(97, 175, 239, 0.25);
  --toggle-active: rgba(97, 175, 239, 0.8);
  --toggle-thumb: #e7edf6;
}

.header {
  position: relative;
  z-index: 2;
}

.header-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 40px;
  align-items: center;
  gap: 12px;
}

.status {
  margin: 4px 0 0;
  font-size: 11px;
  color: var(--app-muted);
}

.status.error {
  color: #c26a5f;
}

.search {
  position: relative;
  flex: 1 1 auto;
  min-width: 0;
}

.search-icon {
  position: absolute;
  left: 14px;
  top: 50%;
  transform: translateY(-50%);
  color: var(--app-muted);
}

.search-icon svg {
  width: 18px;
  height: 18px;
  display: block;
}

.search input {
  width: 100%;
  padding: 12px 42px 12px 42px;
  border-radius: 16px;
  border: 1px solid var(--app-border);
  outline: none;
  font-size: 14px;
  color: var(--app-text);
  background: var(--app-surface);
  box-sizing: border-box;
}

.search input::placeholder {
  color: var(--app-muted);
}

.search-clear {
  position: absolute;
  right: 12px;
  top: 50%;
  transform: translateY(-50%);
  width: 26px;
  height: 26px;
  border-radius: 8px;
  border: 1px solid transparent;
  background: transparent;
  color: var(--app-muted);
  display: grid;
  place-items: center;
  cursor: pointer;
  padding: 0;
  transition: background 120ms ease, color 120ms ease;
}

.search-clear:hover {
  background: var(--app-surface-strong);
  color: var(--app-text);
}

.search-clear svg {
  width: 12px;
  height: 12px;
  display: block;
}

.icon-button {
  width: 40px;
  height: 40px;
  border-radius: 12px;
  border: 1px solid var(--app-border);
  background: var(--app-surface-strong);
  color: var(--app-muted-strong);
  display: grid;
  place-items: center;
  cursor: pointer;
  flex-shrink: 0;
  transition: transform 120ms ease, box-shadow 120ms ease, background 120ms ease;
}

.icon-button:hover {
  transform: scale(1.02);
  box-shadow: 0 6px 14px rgba(0, 0, 0, 0.12);
}

.icon-button svg {
  width: 18px;
  height: 18px;
}

.settings-popover {
  position: absolute;
  right: 0;
  top: calc(100% + 10px);
  width: 250px;
  background: var(--app-surface-strong);
  border: 1px solid var(--app-border);
  border-radius: 14px;
  padding: 14px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  box-shadow: 0 22px 50px rgba(0, 0, 0, 0.18);
  backdrop-filter: blur(20px);
}

.settings-title {
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.18em;
  color: var(--app-muted);
}

.settings-group {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.settings-actions {
  display: flex;
  justify-content: flex-end;
}

.history {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-rows: repeat(5, var(--history-item-height));
  gap: var(--history-gap);
  align-content: start;
  width: calc(100% + 20px);
  margin: 0 -10px;
  padding: 0 0 18px;
  overflow: visible;
  box-sizing: border-box;
  position: relative;
}

.history-item {
  display: grid;
  grid-template-columns: 30px 1fr auto;
  gap: 10px;
  align-items: center;
  height: var(--history-item-height);
  padding: 6px 10px;
  border-radius: 12px;
  background: var(--app-surface);
  border: 1px solid var(--app-border);
  color: inherit;
  cursor: pointer;
  transition: transform 120ms ease, background 120ms ease, border-color 120ms ease;
  text-align: left;
}

.history-item:hover {
  transform: translateY(-1px);
  background: var(--app-surface-strong);
}

.history-item.active {
  border-color: var(--accent);
  background: var(--app-surface-strong);
  box-shadow: 0 10px 24px rgba(0, 0, 0, 0.15);
  animation: selectionPulse 160ms ease;
}

@keyframes selectionPulse {
  0% {
    transform: scale(1);
  }
  50% {
    transform: scale(1.01);
  }
  100% {
    transform: scale(1);
  }
}

.history-index {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
}

.mini-keycap {
  width: 24px;
  height: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 6px;
  background: linear-gradient(180deg, rgba(255, 249, 240, 0.98), rgba(221, 201, 176, 0.96));
  color: var(--keycap-text);
  font-size: 12px;
  font-weight: 700;
  border: 1px solid rgba(121, 99, 73, 0.6);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.85),
    inset 0 -2px 0 rgba(0, 0, 0, 0.1),
    0 2px 0 var(--keycap-shadow),
    0 4px 8px rgba(0, 0, 0, 0.16);
}

.history-content {
  display: grid;
  gap: 4px;
  min-width: 0;
  align-content: center;
}

.history-title {
  font-size: 13px;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.history-subtitle {
  font-size: 11px;
  color: var(--app-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.history-badge {
  display: inline-flex;
  align-items: center;
  justify-self: start;
  width: fit-content;
  padding: 2px 6px;
  border-radius: 999px;
  font-size: 9px;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--app-muted-strong);
  background: var(--app-surface-strong);
  border: 1px solid var(--app-border);
}

.page-indicator {
  display: flex;
  justify-content: center;
  align-items: center;
  gap: 6px;
  padding: 4px 0;
}

.page-indicator--bottom {
  position: absolute;
  bottom: 0;
  left: 50%;
  transform: translateX(-50%);
  pointer-events: none;
}

.page-dot {
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background: rgba(47, 42, 33, 0.25);
  transition: transform 120ms ease, background 120ms ease;
}

.theme-dark .page-dot {
  background: rgba(255, 255, 255, 0.3);
}

.page-dot.active {
  background: var(--accent);
  transform: scale(1.2);
}

.history-meta {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 4px;
}

.history-time {
  font-size: 10px;
  color: var(--app-muted);
}

.history-thumb {
  width: 38px;
  height: 26px;
  object-fit: cover;
  border-radius: 6px;
  border: 1px solid var(--app-border);
}

.empty {
  display: flex;
  flex-direction: column;
  gap: 8px;
  text-align: center;
  color: var(--app-muted);
  align-items: center;
  justify-content: center;
  grid-row: 1 / -1;
}

.footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
  flex-wrap: nowrap;
  padding: 14px 0 8px;
  border-top: 1px solid var(--app-border);
}

.shortcut-block {
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  gap: 12px;
}

.shortcut-label {
  font-size: 10px;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: var(--app-muted);
  line-height: 1.2;
}

.keycaps {
  display: flex;
  justify-content: center;
  align-items: center;
  gap: 6px;
  flex-wrap: nowrap;
}

.keycap {
  padding: 5px 8px;
  min-width: 28px;
  text-align: center;
  font-weight: 600;
  font-size: 10px;
  color: var(--keycap-text);
  border-radius: 7px;
  background: linear-gradient(180deg, rgba(255, 249, 240, 0.95), rgba(227, 210, 186, 0.95));
  border: 1px solid rgba(139, 113, 82, 0.45);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.75),
    inset 0 -2px 0 rgba(0, 0, 0, 0.08),
    0 3px 0 var(--keycap-shadow),
    0 6px 10px rgba(0, 0, 0, 0.18);
  transition: transform 80ms ease, box-shadow 80ms ease;
}

.keycap--wide {
  min-width: 48px;
}

.keycap.pressed {
  transform: translateY(2px);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.7),
    inset 0 -1px 0 rgba(0, 0, 0, 0.08),
    0 1px 0 var(--keycap-shadow),
    0 4px 8px rgba(0, 0, 0, 0.18);
}

.keycap-plus {
  font-size: 12px;
  font-weight: 600;
  color: var(--app-muted-strong);
}

.setting {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  font-size: 12px;
  color: var(--app-muted-strong);
}

.setting input[type="number"] {
  width: 72px;
  border-radius: 10px;
  border: 1px solid var(--app-border);
  padding: 6px 8px;
  background: var(--app-surface);
  color: var(--app-text);
}

.settings-select {
  min-width: 160px;
  border-radius: 10px;
  border: 1px solid var(--app-border);
  padding: 6px 8px;
  background: var(--app-surface);
  color: var(--app-text);
  font-size: 12px;
}

.settings-select:focus {
  outline: none;
  border-color: var(--accent);
}

.toggle {
  position: relative;
  display: inline-flex;
  align-items: center;
}

.toggle input {
  opacity: 0;
  width: 0;
  height: 0;
}

.toggle-slider {
  width: 42px;
  height: 24px;
  background: var(--toggle-bg);
  border-radius: 999px;
  position: relative;
  transition: background 0.2s ease;
}

.toggle-slider::after {
  content: "";
  position: absolute;
  top: 3px;
  left: 3px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: var(--toggle-thumb);
  transition: transform 0.2s ease;
}

.toggle input:checked + .toggle-slider {
  background: var(--toggle-active);
}

.toggle input:checked + .toggle-slider::after {
  transform: translateX(18px);
}

.ghost {
  border: 1px solid var(--app-border);
  background: transparent;
  color: var(--app-text);
  padding: 8px 14px;
  border-radius: 12px;
  font-size: 12px;
  cursor: pointer;
  transition: background 0.2s ease, border 0.2s ease;
}

.ghost:hover {
  background: var(--app-surface);
}
</style>

<style>
:root {
  font-family: "Inter", "SF Pro Text", "SF Pro Display", system-ui, sans-serif;
  font-size: 15px;
  color: #2f2a21;
  background-color: transparent;
  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

body {
  margin: 0;
  background: transparent;
  height: 100vh;
  overflow: hidden;
}

#app {
  height: 100vh;
  padding: 0;
  overflow: hidden;
}
</style>
