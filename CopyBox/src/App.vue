<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref } from "vue";
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

type HistorySettings = {
  maxItems: number;
  autoPaste: boolean;
  capturePaused: boolean;
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
const activeKeys = reactive({ meta: false, shift: false, v: false, number: "" });
const settings = reactive<HistorySettings>({
  maxItems: 50,
  autoPaste: false,
  capturePaused: false,
});
const status = reactive<CaptureStatus>({
  lastCaptureAt: undefined,
  lastError: undefined,
});
let isClosing = false;

const numberKeyLabel = computed(() => activeKeys.number || "1-9");
const primaryModifier = computed(() => (isMac.value ? "⌘" : "Ctrl"));
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

const visibleItems = computed(() => filteredItems.value.slice(0, settings.maxItems));

const friendlyShortcut = computed(() =>
  isMac.value ? "Command + Shift + V" : "Ctrl + Shift + V"
);

const statusMessage = computed(() => {
  if (settings.capturePaused) {
    return "Capture paused";
  }
  if (status.lastError) {
    return status.lastError;
  }
  if (status.lastCaptureAt) {
    return `Last capture ${formatTimeAgo(status.lastCaptureAt)}`;
  }
  return "Waiting for first copy";
});

function applyState(payload: StoredState) {
  items.value = payload.items;
  Object.assign(settings, payload.settings);
  Object.assign(status, payload.status ?? {});
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
  try {
    await appWindow.setAlwaysOnTop(false);
    await appWindow.hide();
  } finally {
    isClosing = false;
  }
}

async function selectItem(item: ClipboardItem) {
  const payload = await invoke<StoredState>("select_item", { id: item.id });
  applyState(payload);
  await closeOverlay();
}

async function updateSettings() {
  const payload = await invoke<StoredState>("update_settings", {
    max_items: settings.maxItems,
    auto_paste: settings.autoPaste,
  });
  applyState(payload);
}

async function toggleCapture() {
  const payload = await invoke<StoredState>("toggle_capture");
  applyState(payload);
}

async function clearHistory() {
  const payload = await invoke<StoredState>("clear_history");
  applyState(payload);
}

function handleKeydown(event: KeyboardEvent) {
  if (!isOpen.value) {
    return;
  }

  if (event.key === "Escape") {
    event.preventDefault();
    void closeOverlay();
    return;
  }

  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "w") {
    event.preventDefault();
    void closeOverlay();
    return;
  }

  if (event.key === "Meta" || event.key === "Control") {
    activeKeys.meta = true;
  }

  if (event.key === "Shift") {
    activeKeys.shift = true;
  }

  if (event.key.toLowerCase() === "v") {
    activeKeys.v = true;
  }

  if (/^[1-9]$/.test(event.key) && search.value.trim().length === 0) {
    const index = Number(event.key) - 1;
    const item = visibleItems.value[index];
    if (item) {
      activeKeys.number = event.key;
      setTimeout(() => {
        activeKeys.number = "";
      }, 140);
      event.preventDefault();
      void selectItem(item);
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
}

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
      return "Files";
    }
    if (item.paths.length === 1) {
      return fileName(item.paths[0]);
    }
    return `${item.paths.length} files`;
  }

  if (item.kind === "image") {
    const dimensions = item.width && item.height ? `${item.width}×${item.height}` : "";
    return `Image ${dimensions}`.trim();
  }

  return "Clipboard";
}

function itemSubtitle(item: ClipboardItem) {
  if (item.kind === "text") {
    return "Text";
  }

  if (item.kind === "link") {
    return "Link";
  }

  if (item.kind === "files") {
    if (!item.paths || item.paths.length === 0) {
      return "File list";
    }
    return item.paths.slice(0, 2).map(fileName).join(" · ");
  }

  if (item.kind === "image") {
    return "Image";
  }

  return "Clipboard";
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

onMounted(async () => {
  isMac.value = detectMac();
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

  if (import.meta.env.DEV) {
    await openOverlay();
  } else {
    await closeOverlay();
  }
});

onUnmounted(() => {
  window.removeEventListener("keydown", handleKeydown);
  window.removeEventListener("keyup", handleKeyup);
  unlistenHistory.value?.();
  unlistenOpen.value?.();
  unlistenClose.value?.();
  unlistenFocus.value?.();
});
</script>

<template>
  <div class="backdrop" v-if="isOpen" @click="closeOverlay" />
  <div class="app" :class="{ 'is-open': isOpen }">
    <header class="header">
      <div class="search">
        <input
          ref="searchInput"
          v-model="search"
          type="text"
          placeholder="Search clipboard history"
        />
      </div>
      <div class="shortcut">
        <div class="keycaps">
          <span class="keycap" :class="{ pressed: activeKeys.meta }">
            {{ primaryModifier }}
          </span>
          <span class="keycap" :class="{ pressed: activeKeys.shift }">Shift</span>
          <span class="keycap" :class="{ pressed: activeKeys.v }">V</span>
          <span class="keycap keycap--wide" :class="{ pressed: activeKeys.number }">
            {{ numberKeyLabel }}
          </span>
        </div>
        <p class="shortcut-copy">{{ friendlyShortcut }} then 1-9</p>
      </div>
    </header>
    <p class="status" :class="{ error: status.lastError }">{{ statusMessage }}</p>

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
        @click="selectItem(item)"
      >
        <div class="history-index">
          <span class="mini-keycap">{{ index + 1 }}</span>
        </div>
        <div class="history-content">
          <div class="history-title">{{ itemTitle(item) }}</div>
          <div class="history-subtitle">{{ itemSubtitle(item) }}</div>
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
    </section>

    <footer class="footer">
      <div class="settings-block">
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
      </div>
      <div class="settings-block">
        <button class="ghost" type="button" @click="toggleCapture">
          {{ settings.capturePaused ? "Resume capture" : "Pause capture" }}
        </button>
      </div>
    </footer>
  </div>
</template>

<style scoped>
.backdrop {
  position: fixed;
  inset: 0;
  background: rgba(8, 8, 12, 0.5);
}

.app {
  position: relative;
  z-index: 1;
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  gap: 20px;
  padding: 28px;
  background: rgba(20, 20, 24, 0.88);
  color: #f6f4ef;
  border-radius: 24px;
  box-shadow: 0 30px 90px rgba(0, 0, 0, 0.55);
  backdrop-filter: blur(28px);
}

.header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 20px;
}

.status {
  margin: 10px 0 0;
  font-size: 12px;
  color: rgba(255, 255, 255, 0.55);
}

.status.error {
  color: #f4b1a6;
}

.shortcut {
  text-align: right;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.keycaps {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  flex-wrap: wrap;
}

.keycap {
  padding: 8px 14px;
  min-width: 38px;
  text-align: center;
  font-weight: 600;
  color: #3c372f;
  border-radius: 10px;
  background: linear-gradient(160deg, #f7f3eb, #d7cebf);
  border: 1px solid #c1b7a7;
  box-shadow: 0 6px 0 #b1a795, 0 14px 18px rgba(0, 0, 0, 0.35);
  transition: transform 80ms ease, box-shadow 80ms ease;
}

.keycap--wide {
  min-width: 60px;
}

.keycap.pressed {
  transform: translateY(4px);
  box-shadow: 0 2px 0 #b1a795, 0 6px 12px rgba(0, 0, 0, 0.35);
}

.shortcut-copy {
  font-size: 12px;
  color: rgba(255, 255, 255, 0.6);
  margin: 0;
}

.search {
  flex: 1;
}

.search input {
  width: 100%;
  padding: 12px 16px;
  border-radius: 14px;
  border: none;
  outline: none;
  font-size: 14px;
  color: #f4f0e8;
  background: rgba(255, 255, 255, 0.08);
}

.search input::placeholder {
  color: rgba(255, 255, 255, 0.45);
}

.history {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 12px;
  overflow: auto;
  padding-right: 4px;
}

.history-item {
  display: grid;
  grid-template-columns: auto 1fr auto;
  gap: 16px;
  align-items: center;
  padding: 14px 16px;
  border-radius: 16px;
  background: rgba(255, 255, 255, 0.06);
  border: 1px solid rgba(255, 255, 255, 0.08);
  color: inherit;
  cursor: pointer;
  transition: transform 120ms ease, background 120ms ease;
  text-align: left;
}

.history-item:hover {
  transform: translateY(-1px);
  background: rgba(255, 255, 255, 0.12);
}

.history-index {
  display: flex;
  align-items: center;
}

.mini-keycap {
  padding: 6px 10px;
  border-radius: 8px;
  background: linear-gradient(160deg, #f7f3eb, #d7cebf);
  color: #3c372f;
  font-weight: 600;
  box-shadow: 0 4px 0 #b1a795, 0 8px 12px rgba(0, 0, 0, 0.35);
}

.history-content {
  display: grid;
  gap: 4px;
}

.history-title {
  font-size: 15px;
  font-weight: 600;
}

.history-subtitle {
  font-size: 12px;
  color: rgba(255, 255, 255, 0.6);
}

.history-badge {
  font-size: 11px;
  letter-spacing: 0.2em;
  text-transform: uppercase;
  color: rgba(255, 255, 255, 0.45);
}

.history-meta {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 8px;
}

.history-time {
  font-size: 11px;
  color: rgba(255, 255, 255, 0.45);
}

.history-thumb {
  width: 44px;
  height: 32px;
  object-fit: cover;
  border-radius: 8px;
  border: 1px solid rgba(255, 255, 255, 0.2);
}

.empty {
  display: flex;
  flex-direction: column;
  gap: 8px;
  text-align: center;
  color: rgba(255, 255, 255, 0.6);
  padding: 30px 0;
}

.footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
  flex-wrap: wrap;
}

.settings-block {
  display: flex;
  gap: 16px;
  align-items: center;
}

.setting {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 12px;
  color: rgba(255, 255, 255, 0.7);
}

.setting input[type="number"] {
  width: 72px;
  border-radius: 10px;
  border: none;
  padding: 6px 8px;
  background: rgba(255, 255, 255, 0.08);
  color: #f4f0e8;
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
  background: rgba(255, 255, 255, 0.2);
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
  background: #f7f3eb;
  transition: transform 0.2s ease;
}

.toggle input:checked + .toggle-slider {
  background: rgba(249, 187, 96, 0.75);
}

.toggle input:checked + .toggle-slider::after {
  transform: translateX(18px);
}

.ghost {
  border: 1px solid rgba(255, 255, 255, 0.2);
  background: transparent;
  color: rgba(255, 255, 255, 0.8);
  padding: 8px 14px;
  border-radius: 12px;
  font-size: 12px;
  cursor: pointer;
  transition: background 0.2s ease;
}

.ghost:hover {
  background: rgba(255, 255, 255, 0.1);
}
</style>

<style>
:root {
  font-family: "Inter", "SF Pro Text", "SF Pro Display", system-ui, sans-serif;
  font-size: 15px;
  color: #f6f4ef;
  background-color: transparent;
  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

body {
  margin: 0;
  background: transparent;
}

#app {
  min-height: 100vh;
  padding: 18px;
}
</style>
