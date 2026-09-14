<template>
  <div class="page">
    <div v-if="accounts.length === 0" class="empty-state card">
      <Icon name="inbox" :size="40" />
      <p>还没有存储，先去 <a href="#" @click.prevent="$emit('go-accounts')">存储管理</a> 添加一个云盘账号</p>
    </div>

    <template v-else>
      <div class="toolbar">
        <div class="storage-select">
          <select class="input" :value="currentId" @change="$emit('switch-account', $event.target.value)">
            <option v-for="a in accounts" :key="a.id" :value="a.id">{{ a.name }}</option>
          </select>
        </div>

        <nav class="crumbs">
          <a href="#" class="crumb-home" @click.prevent="$emit('goto', 0)">
            <Icon name="home" :size="15" />
          </a>
          <template v-for="(c, i) in crumbs" :key="c.fid + i">
            <Icon name="chevron-right" :size="14" class="crumb-sep" />
            <a
              v-if="i !== crumbs.length - 1"
              href="#"
              class="crumb-link"
              @click.prevent="$emit('goto', i)"
              >{{ c.name }}</a
            >
            <span v-else class="crumb-current">{{ c.name }}</span>
          </template>
        </nav>

        <div class="toolbar-actions">
          <button class="btn-icon btn-ghost" title="刷新" @click="$emit('refresh')">
            <Icon name="refresh" :size="17" />
          </button>
          <div class="view-toggle">
            <button :class="{ active: viewMode === 'list' }" @click="$emit('update:view-mode', 'list')" title="列表视图">
              <Icon name="list" :size="16" />
            </button>
            <button :class="{ active: viewMode === 'grid' }" @click="$emit('update:view-mode', 'grid')" title="宫格视图">
              <Icon name="grid" :size="16" />
            </button>
          </div>
        </div>
      </div>

      <div v-if="err" class="alert alert-error">{{ err }}</div>

      <!-- 加载中 -->
      <div v-if="loading" class="state-box card">
        <span class="spin loader-lg"></span>
        <span>加载中…</span>
      </div>

      <!-- 空目录 -->
      <div v-else-if="entries.length === 0" class="state-box card">
        <Icon name="inbox" :size="34" />
        <span>此文件夹为空</span>
      </div>

      <!-- 列表视图 -->
      <div v-else-if="viewMode === 'list'" class="card table-card">
        <table>
          <thead>
            <tr>
              <th class="col-name">文件名</th>
              <th class="col-size">大小</th>
              <th class="col-date">修改时间</th>
              <th class="col-op"></th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="e in entries" :key="e.fid" @dblclick="rowActivate(e)">
              <td class="col-name">
                <a
                  v-if="e.is_dir"
                  href="#"
                  class="fname"
                  @click.prevent="$emit('open-dir', e)"
                >
                  <span class="ficon"><FileIcon :name="e.name" :is-dir="true" /></span>
                  {{ e.name }}
                </a>
                <span v-else class="fname">
                  <span class="ficon"><FileIcon :name="e.name" :is-dir="false" /></span>
                  {{ e.name }}
                </span>
              </td>
              <td class="col-size">{{ e.is_dir ? '-' : fmtSize(e.size) }}</td>
              <td class="col-date">{{ fmtDate(e.updated_at) }}</td>
              <td class="col-op">
                <button v-if="isVideo(e)" class="btn-icon btn-ghost" title="播放" @click="$emit('play', e)">
                  <Icon name="play" :size="15" />
                </button>
                <button v-if="!e.is_dir" class="btn-icon btn-ghost" title="下载" @click="$emit('download', e)">
                  <Icon name="download" :size="15" />
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <!-- 宫格视图 -->
      <div v-else class="grid-view">
        <div
          v-for="e in entries"
          :key="e.fid"
          class="grid-item"
          @click="e.is_dir ? $emit('open-dir', e) : (isVideo(e) ? $emit('play', e) : $emit('download', e))"
        >
          <div class="grid-icon"><FileIcon :name="e.name" :is-dir="e.is_dir" /></div>
          <div class="grid-name" :title="e.name">{{ e.name }}</div>
          <div class="grid-meta">{{ e.is_dir ? '文件夹' : fmtSize(e.size) }}</div>
          <div class="grid-hover-actions" v-if="!e.is_dir">
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
  </div>
</template>

<script setup>
import Icon from './Icon.vue'
import FileIcon from './FileIcon.vue'

const props = defineProps({
  accounts: { type: Array, required: true },
  currentId: { type: String, required: true },
  crumbs: { type: Array, required: true },
  entries: { type: Array, required: true },
  loading: { type: Boolean, default: false },
  err: { type: String, default: '' },
  viewMode: { type: String, default: 'list' }
})
const emit = defineEmits([
  'go-accounts', 'switch-account', 'goto', 'refresh', 'update:view-mode',
  'open-dir', 'play', 'download'
])

const VIDEO_EXT = ['mp4', 'mkv', 'webm', 'mov', 'm4v', 'avi', 'flv', 'ts', 'wmv', 'rmvb', '3gp']
const isVideo = (e) => !e.is_dir && VIDEO_EXT.includes(e.name.split('.').pop()?.toLowerCase())

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
  color: var(--ol-text-dim);
}
.crumb-home:hover {
  color: var(--ol-primary);
}
.crumb-sep {
  color: var(--ol-text-faint);
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
