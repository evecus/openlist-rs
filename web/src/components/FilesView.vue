<template>
  <div class="page">
    <div v-if="accounts.length === 0" class="empty-state card">
      <Icon name="inbox" :size="40" />
      <p>还没有存储，先去 <a href="#" @click.prevent="$emit('go-accounts')">存储管理</a> 添加一个云盘账号</p>
    </div>

    <template v-else>
      <div class="toolbar">
        <nav class="crumbs">
          <a href="#" class="crumb-home" title="返回网盘列表" @click.prevent="$emit('go-home')">
            <Icon name="home" :size="16" />
            <span>首页</span>
          </a>
          <template v-if="currentId">
            <template v-for="(c, i) in crumbs" :key="c.fid + i">
              <span class="crumb-sep">/</span>
              <a
                v-if="playing || i !== crumbs.length - 1"
                href="#"
                class="crumb-link"
                @click.prevent="$emit('goto', i)"
                >{{ c.name }}</a
              >
              <span v-else class="crumb-current">{{ c.name }}</span>
            </template>
            <template v-if="playing">
              <span class="crumb-sep">/</span>
              <span class="crumb-current">{{ playing.name }}</span>
            </template>
          </template>
        </nav>
      </div>

      <div v-if="err" class="alert alert-error">{{ err }}</div>

      <!-- 视频播放（与列表同页，仿 OpenList 视频预览） -->
      <div v-if="playing" class="card player-card">
        <video :src="playing._url" controls autoplay class="player-video"></video>
        <div class="player-bar">
          <span class="player-fname" :title="playing.name">{{ playing.name }}</span>
        </div>
        <div class="player-exts">
          <button
            v-for="p in PLAYERS"
            :key="p.name"
            class="ext-btn"
            :style="{ background: p.color }"
            :title="'用 ' + p.name + ' 播放'"
            @click="openExternal(p)"
          >
            <Icon name="play" :size="14" />
          </button>
          <button class="ext-btn ext-open" title="打开原始链接" @click="openRaw">
            <Icon name="arrow-right" :size="17" />
          </button>
        </div>
      </div>

      <template v-else>

      <!-- 加载中（进入网盘后） -->
      <div v-if="currentId && loading" class="state-box card">
        <span class="spin loader-lg"></span>
        <span>加载中…</span>
      </div>

      <!-- 空目录 -->
      <div v-else-if="currentId && entries.length === 0" class="state-box card">
        <Icon name="inbox" :size="34" />
        <span>此文件夹为空</span>
      </div>

      <!-- 统一列表视图：根目录显示网盘列表，进入网盘后显示文件列表 -->
      <div v-else-if="viewMode === 'list'" class="card table-card">
        <table>
          <thead>
            <tr>
              <th class="col-name">名称</th>
              <th class="col-size">大小</th>
              <th class="col-date">修改时间</th>
              <th class="col-op"></th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="e in displayEntries"
              :key="e.key"
              :class="{ 'drive-row': e.is_drive }"
              @click="e.is_drive && $emit('open-account', e.id)"
              @dblclick="!e.is_drive && rowActivate(e)"
            >
              <td class="col-name">
                <a v-if="e.is_dir && !e.is_drive" href="#" class="fname" @click.prevent="$emit('open-dir', e)">
                  <span class="ficon"><FileIcon :name="e.name" :is-dir="true" /></span>
                  {{ e.name }}
                </a>
                <span v-else class="fname">
                  <span class="ficon" :class="{ 'drive-icon': e.is_drive }">
                    <Icon v-if="e.is_drive" name="cloud" :size="19" />
                    <FileIcon v-else :name="e.name" :is-dir="false" />
                  </span>
                  {{ e.name }}
                  <span v-if="e.is_drive" class="drive-type">{{ driverLabels[e.driver] || e.driver }}</span>
                </span>
              </td>
              <td class="col-size">{{ e.is_dir || e.is_drive ? '-' : fmtSize(e.size) }}</td>
              <td class="col-date">{{ e.updated_at ? fmtDate(e.updated_at) : '-' }}</td>
              <td class="col-op">
                <template v-if="e.is_drive">
                  <Icon name="chevron-right" :size="16" class="drive-go" />
                </template>
                <template v-else>
                  <button v-if="isVideo(e)" class="btn-icon btn-ghost" title="播放" @click="$emit('play', e)">
                    <Icon name="play" :size="15" />
                  </button>
                  <button v-if="!e.is_dir" class="btn-icon btn-ghost" title="下载" @click="$emit('download', e)">
                    <Icon name="download" :size="15" />
                  </button>
                </template>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <!-- 统一宫格视图 -->
      <div v-else class="grid-view">
        <div
          v-for="e in displayEntries"
          :key="e.key"
          class="grid-item"
          @click="e.is_drive ? $emit('open-account', e.id) : (e.is_dir ? $emit('open-dir', e) : (isVideo(e) ? $emit('play', e) : $emit('download', e)))"
        >
          <div class="grid-icon">
            <Icon v-if="e.is_drive" name="cloud" :size="32" class="grid-drive-icon" />
            <FileIcon v-else :name="e.name" :is-dir="e.is_dir" />
          </div>
          <div class="grid-name" :title="e.name">{{ e.name }}</div>
          <div class="grid-meta">{{ e.is_drive ? (driverLabels[e.driver] || e.driver) : (e.is_dir ? '文件夹' : fmtSize(e.size)) }}</div>
          <div class="grid-hover-actions" v-if="!e.is_drive && !e.is_dir">
            <button v-if="isVideo(e)" class="btn-icon btn-ghost sm" title="播放" @click.stop="$emit('play', e)">
              <Icon name="play" :size="13" />
            </button>
            <button class="btn-icon btn-ghost sm" title="下载" @click.stop="$emit('download', e)">
              <Icon name="download" :size="13" />
            </button>
          </div>
        </div>
      </div>
      </template>
    </template>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import Icon from './Icon.vue'
import FileIcon from './FileIcon.vue'

const props = defineProps({
  accounts: { type: Array, required: true },
  currentId: { type: String, required: true },
  crumbs: { type: Array, required: true },
  entries: { type: Array, required: true },
  playing: { type: Object, default: null },
  loading: { type: Boolean, default: false },
  err: { type: String, default: '' },
  viewMode: { type: String, default: 'list' },
  driverLabels: { type: Object, default: () => ({}) }
})
const emit = defineEmits([
  'go-accounts', 'go-home', 'open-account', 'switch-account', 'goto', 'refresh', 'update:view-mode',
  'open-dir', 'play', 'download'
])

// 统一数据源：根目录（未选网盘）时把网盘映射为“文件夹”行，进入网盘后为文件条目
const displayEntries = computed(() => {
  if (props.currentId) {
    return props.entries.map((e) => ({ ...e, key: e.fid }))
  }
  return props.accounts.map((a) => ({
    key: 'drive-' + a.id,
    id: a.id,
    name: a.name,
    is_dir: true,
    is_drive: true,
    driver: a.driver,
    size: null,
    updated_at: null
  }))
})

const VIDEO_EXT = ['mp4', 'mkv', 'webm', 'mov', 'm4v', 'avi', 'flv', 'ts', 'wmv', 'rmvb', '3gp']
const isVideo = (e) => !e.is_dir && VIDEO_EXT.includes(e.name.split('.').pop()?.toLowerCase())

// 外部播放器 URL Scheme（仿 OpenList 视频页的一排播放器图标）
const PLAYERS = [
  { name: 'PotPlayer', color: '#f5c518', url: (u) => `potplayer://${u}` },
  { name: 'VLC', color: '#f5792a', url: (u) => `vlc://${u}` },
  { name: 'MX Player', color: '#2f9cf4', url: (u) => `intent:${encodeURIComponent(u)}#Intent;package=com.mxtech.videoplayer.ad;end` },
  { name: 'IINA', color: '#7c6cf0', url: (u) => `iina://weblink?url=${encodeURIComponent(u)}` },
  { name: 'nPlayer', color: '#6e3fa3', url: (u) => `nplayer-${u}` },
  { name: 'Infuse', color: '#ff7043', url: (u) => `infuse://x-callback-url/play?url=${encodeURIComponent(u)}` }
]

function absolutePlayUrl() {
  try {
    return new URL(props.playing._url, window.location.href).href
  } catch {
    return props.playing._url
  }
}

function openExternal(p) {
  window.location.href = p.url(absolutePlayUrl())
}

function openRaw() {
  window.open(absolutePlayUrl(), '_blank')
}

function rowActivate(e) {
  if (e.is_dir) emit('open-dir', e)
  else if (isVideo(e)) emit('play', e)
}

function fmtSize(n) {
  if (n == null) return '-'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let i = 0
  let v = n
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${units[i]}`
}

function fmtDate(ms) {
  if (!ms) return '-'
  const d = new Date(ms)
  const p = (x) => String(x).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}
</script>

<style scoped>
.page {
  max-width: 1180px;
  margin: 0 auto;
  padding: 20px 20px 60px;
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  color: var(--ol-text-dim);
  padding: 60px 20px;
  text-align: center;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 14px;
  margin-bottom: 14px;
  flex-wrap: wrap;
  min-height: 36px;
}
.storage-select select {
  width: auto;
  min-width: 170px;
  height: 36px;
  font-weight: 500;
}
.crumbs {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-wrap: wrap;
  flex: 1;
  min-width: 120px;
  color: var(--ol-text-dim);
}
.crumb-home {
  display: flex;
  align-items: center;
  gap: 5px;
  color: var(--ol-text-dim);
  font-size: 13.5px;
}
.crumb-home:hover {
  color: var(--ol-primary);
}
.crumb-sep {
  color: var(--ol-text-faint);
  font-size: 13.5px;
  padding: 0 3px;
}
.crumb-link {
  color: var(--ol-text-dim);
  font-size: 13.5px;
}
.crumb-link:hover {
  color: var(--ol-primary);
  text-decoration: none;
}
.crumb-current {
  color: var(--ol-text);
  font-weight: 500;
  font-size: 13.5px;
}

.toolbar-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-left: auto;
}
.view-toggle {
  display: flex;
  border: 1px solid var(--ol-border-strong);
  border-radius: var(--ol-radius-sm);
  overflow: hidden;
}
.view-toggle button {
  width: 32px;
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--ol-panel);
  border: none;
  color: var(--ol-text-dim);
  cursor: pointer;
}
.view-toggle button + button {
  border-left: 1px solid var(--ol-border-strong);
}
.view-toggle button.active {
  background: var(--ol-primary-light);
  color: var(--ol-primary);
}

.state-box {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 60px 0;
  color: var(--ol-text-dim);
  font-size: 13.5px;
}
.loader-lg {
  width: 26px;
  height: 26px;
  border: 3px solid var(--ol-border-strong);
  border-top-color: var(--ol-primary);
  border-radius: 50%;
}

/* 列表视图 */
.table-card {
  overflow: hidden;
}
table {
  width: 100%;
  border-collapse: collapse;
}
th,
td {
  text-align: left;
  padding: 11px 18px;
  border-bottom: 1px solid var(--ol-border);
}
th {
  color: var(--ol-text-dim);
  font-size: 12.5px;
  font-weight: 600;
  background: var(--ol-bg);
}
tbody tr:last-child td {
  border-bottom: none;
}
tbody tr:hover {
  background: var(--ol-primary-light);
}
.col-name {
  width: 56%;
}
.col-size,
.col-date {
  color: var(--ol-text-dim);
  white-space: nowrap;
  font-size: 13px;
}
.col-op {
  white-space: nowrap;
  text-align: right;
}
.col-op button {
  margin-left: 2px;
}
.fname {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  overflow-wrap: anywhere;
  color: var(--ol-text);
}
a.fname:hover {
  color: var(--ol-primary);
  text-decoration: none;
}
.ficon {
  width: 20px;
  height: 20px;
  flex-shrink: 0;
}

/* 网盘行（根目录） */
.drive-row {
  cursor: pointer;
}
.drive-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: 9px;
  background: var(--ol-primary-light);
  color: var(--ol-primary);
}
.drive-type {
  margin-left: 8px;
  padding: 1px 8px;
  font-size: 11px;
  color: var(--ol-text-faint);
  background: var(--ol-bg);
  border: 1px solid var(--ol-border);
  border-radius: 999px;
}
.drive-go {
  color: var(--ol-text-faint);
}

/* 视频播放（同页预览） */
.player-card {
  padding: 0;
  overflow: hidden;
}
.player-video {
  display: block;
  width: 100%;
  aspect-ratio: 16 / 9;
  background: #000;
  outline: none;
}
.player-bar {
  padding: 10px 12px;
  border-top: 1px solid var(--ol-border);
}
.player-fname {
  display: block;
  max-width: 100%;
  padding: 8px 12px;
  background: var(--ol-bg);
  border: 1px solid var(--ol-border);
  border-radius: 8px;
  font-size: 13px;
  color: var(--ol-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.player-exts {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 14px;
  padding: 10px 12px 14px;
}
.ext-btn {
  width: 34px;
  height: 34px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 50%;
  color: #fff;
  cursor: pointer;
  transition: transform 0.15s ease, box-shadow 0.15s ease;
}
.ext-btn:hover {
  transform: scale(1.12);
  box-shadow: var(--ol-shadow);
}
.ext-open {
  background: var(--ol-primary);
}

/* 宫格视图 */
.grid-view {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
  gap: 6px;
}
.grid-item {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 16px 8px 12px;
  border-radius: var(--ol-radius-sm);
  cursor: pointer;
  text-align: center;
}
.grid-item:hover {
  background: var(--ol-primary-light);
}
.grid-icon {
  width: 46px;
  height: 46px;
}
.grid-name {
  font-size: 12.5px;
  width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--ol-text);
}
.grid-meta {
  font-size: 11px;
  color: var(--ol-text-dim);
}
.grid-hover-actions {
  position: absolute;
  top: 6px;
  right: 6px;
  display: none;
  gap: 2px;
}
.grid-item:hover .grid-hover-actions {
  display: flex;
}
.btn-icon.sm {
  width: 26px;
  height: 26px;
  background: var(--ol-panel);
  border: 1px solid var(--ol-border);
}

@media (max-width: 640px) {
  .page {
    padding: 14px 12px 40px;
  }
  .toolbar {
    gap: 10px;
  }
  .storage-select select {
    min-width: 0;
    width: 100%;
  }
  .storage-select {
    flex: 1 1 100%;
    order: 1;
  }
  .crumbs {
    order: 3;
    flex: 1 1 100%;
  }
  .toolbar-actions {
    order: 2;
    margin-left: 0;
  }
  th,
  td {
    padding: 10px 10px;
  }
  .col-date {
    display: none;
  }
  .col-name {
    width: auto;
  }
}
</style>
