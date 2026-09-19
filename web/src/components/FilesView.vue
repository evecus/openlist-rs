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
                v-if="preview || i !== crumbs.length - 1"
                href="#"
                class="crumb-link"
                @click.prevent="$emit('goto', i)"
                >{{ c.name }}</a
              >
              <span v-else class="crumb-current">{{ c.name }}</span>
            </template>
            <template v-if="preview">
              <span class="crumb-sep">/</span>
              <span class="crumb-current">{{ preview.name }}</span>
            </template>
          </template>
        </nav>

      </div>

      <div v-if="err" class="alert alert-error">{{ err }}</div>

      <!-- 统一预览卡片（视频 / 音乐 / 图片 / PDF / 文本，与列表同页） -->
      <div v-if="preview" class="card player-card">
        <div class="player-head">
          <span class="player-kind" :title="KIND_TITLE[preview._kind]">
            <Icon :name="KIND_ICON[preview._kind]" :size="14" />
          </span>
          <span class="player-fname" :title="preview.name">{{ preview.name }}</span>
          <span v-if="navList.length > 1" class="player-pos">{{ navIndex + 1 }} / {{ navList.length }}</span>
          <div class="player-nav">
            <button
              class="btn-icon btn-ghost"
              :disabled="navIndex <= 0"
              title="上一个（←）"
              @click="nav(-1)"
            >
              <Icon name="chevron-left" :size="16" />
            </button>
            <button
              class="btn-icon btn-ghost"
              :disabled="navIndex >= navList.length - 1"
              title="下一个（→）"
              @click="nav(1)"
            >
              <Icon name="chevron-right" :size="16" />
            </button>
            <button class="btn-icon btn-ghost" title="关闭（Esc）" @click="$emit('close-preview')">
              <Icon name="close" :size="16" />
            </button>
          </div>
        </div>

        <!-- 视频 -->
        <video
          v-if="preview._kind === 'video'"
          :src="preview._url"
          controls
          autoplay
          class="player-video"
        ></video>

        <!-- 音乐 -->
        <div v-else-if="preview._kind === 'audio'" class="audio-stage">
          <div class="disc" :class="{ spinning: audioPlaying }">
            <Icon name="music" :size="44" />
          </div>
          <audio
            :src="preview._url"
            controls
            autoplay
            class="audio-player"
            @play="audioPlaying = true"
            @pause="audioPlaying = false"
            @ended="audioPlaying = false"
          ></audio>
        </div>

        <!-- 图片 -->
        <div v-else-if="preview._kind === 'image'" class="image-stage">
          <img :src="preview._url" :alt="preview.name" class="image-view" />
        </div>

        <!-- PDF -->
        <iframe
          v-else-if="preview._kind === 'pdf'"
          :src="preview._url"
          class="pdf-frame"
          title="PDF 预览"
        ></iframe>

        <!-- 文本 / Markdown / 代码等 -->
        <div v-else-if="preview._kind === 'text'" class="text-stage">
          <div v-if="textLoading" class="state-box">
            <span class="spin loader-lg"></span>
            <span>加载中…</span>
          </div>
          <div v-else-if="textError" class="state-box">
            <Icon name="alert" :size="30" />
            <span>{{ textError }}</span>
            <a :href="preview._url" target="_blank" class="btn btn-secondary" rel="noopener">在新窗口打开</a>
          </div>
          <pre v-else class="text-view">{{ textContent }}</pre>
        </div>

        <div class="player-exts">
          <template v-if="preview._kind === 'video' || preview._kind === 'audio'">
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
          </template>
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
        <!-- 桌面端：传统表格 -->
        <table class="desktop-table">
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
                  <span class="fname-text">{{ e.name }}</span>
                </a>
                <span v-else class="fname">
                  <span class="ficon" :class="{ 'drive-icon': e.is_drive }">
                    <Icon v-if="e.is_drive" name="cloud" :size="19" />
                    <FileIcon v-else :name="e.name" :is-dir="false" />
                  </span>
                  <span class="fname-text">{{ e.name }}</span>
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
                  <button
                    v-if="kindOf(e.name, e.is_dir)"
                    class="btn-icon btn-ghost"
                    :title="KIND_TITLE[kindOf(e.name, e.is_dir)]"
                    @click="$emit('preview', e)"
                  >
                    <Icon :name="KIND_ICON[kindOf(e.name, e.is_dir)]" :size="15" />
                  </button>
                  <button v-if="!e.is_dir" class="btn-icon btn-ghost" title="下载" @click="$emit('download', e)">
                    <Icon name="download" :size="15" />
                  </button>
                  <template v-if="canWrite">
                    <button class="btn-icon btn-ghost" title="重命名" @click="$emit('rename', e)">
                      <Icon name="edit" :size="15" />
                    </button>
                    <button class="btn-icon btn-ghost" title="移动到…" @click="$emit('move', e)">
                      <Icon name="move" :size="15" />
                    </button>
                    <button v-if="!e.is_dir" class="btn-icon btn-ghost" title="复制到…" @click="$emit('copy', e)">
                      <Icon name="copy" :size="15" />
                    </button>
                    <button class="btn-icon btn-ghost op-del" title="删除" @click="$emit('remove', e)">
                      <Icon name="trash" :size="15" />
                    </button>
                  </template>
                </template>
              </td>
            </tr>
          </tbody>
        </table>

        <!-- 移动端：卡片式列表 -->
        <ul class="mobile-list">
          <li
            v-for="e in displayEntries"
            :key="e.key"
            class="mobile-item"
            :class="{ 'drive-row': e.is_drive }"
            @click="e.is_drive && $emit('open-account', e.id)"
          >
            <!-- 第一行：图标 + 名称 + 大小 + 时间 -->
            <div class="mobile-row-info">
              <span class="ficon" :class="{ 'drive-icon': e.is_drive }">
                <Icon v-if="e.is_drive" name="cloud" :size="20" />
                <FileIcon v-else :name="e.name" :is-dir="e.is_dir" />
              </span>
              <a
                v-if="e.is_dir && !e.is_drive"
                href="#"
                class="mobile-fname"
                @click.prevent="$emit('open-dir', e)"
              >{{ e.name }}</a>
              <span v-else class="mobile-fname">
                {{ e.name }}
                <span v-if="e.is_drive" class="drive-type">{{ driverLabels[e.driver] || e.driver }}</span>
              </span>
              <span class="mobile-meta">
                <span>{{ e.is_dir || e.is_drive ? '-' : fmtSize(e.size) }}</span>
                <span v-if="e.updated_at" class="mobile-date">{{ fmtDate(e.updated_at) }}</span>
              </span>
            </div>
            <!-- 第二行：操作按钮，右对齐 -->
            <div v-if="!e.is_drive" class="mobile-row-ops">
              <button
                v-if="kindOf(e.name, e.is_dir)"
                class="btn-icon btn-ghost"
                :title="KIND_TITLE[kindOf(e.name, e.is_dir)]"
                @click.stop="$emit('preview', e)"
              >
                <Icon :name="KIND_ICON[kindOf(e.name, e.is_dir)]" :size="15" />
              </button>
              <button v-if="!e.is_dir" class="btn-icon btn-ghost" title="下载" @click.stop="$emit('download', e)">
                <Icon name="download" :size="15" />
              </button>
              <template v-if="canWrite">
                <button class="btn-icon btn-ghost" title="重命名" @click.stop="$emit('rename', e)">
                  <Icon name="edit" :size="15" />
                </button>
                <button class="btn-icon btn-ghost" title="移动到…" @click.stop="$emit('move', e)">
                  <Icon name="move" :size="15" />
                </button>
                <button v-if="!e.is_dir" class="btn-icon btn-ghost" title="复制到…" @click.stop="$emit('copy', e)">
                  <Icon name="copy" :size="15" />
                </button>
                <button class="btn-icon btn-ghost op-del" title="删除" @click.stop="$emit('remove', e)">
                  <Icon name="trash" :size="15" />
                </button>
              </template>
              <Icon v-if="e.is_drive" name="chevron-right" :size="16" class="drive-go" />
            </div>
          </li>
        </ul>
      </div>

      <!-- 统一宫格视图 -->
      <div v-else class="grid-view">
        <div
          v-for="e in displayEntries"
          :key="e.key"
          class="grid-item"
          @click="gridActivate(e)"
        >
          <div class="grid-icon">
            <Icon v-if="e.is_drive" name="cloud" :size="32" class="grid-drive-icon" />
            <FileIcon v-else :name="e.name" :is-dir="e.is_dir" />
          </div>
          <div class="grid-name" :title="e.name">{{ e.name }}</div>
          <div class="grid-meta">{{ e.is_drive ? (driverLabels[e.driver] || e.driver) : (e.is_dir ? '文件夹' : fmtSize(e.size)) }}</div>
          <div class="grid-hover-actions" v-if="!e.is_drive">
            <button
              v-if="!e.is_dir && kindOf(e.name, e.is_dir)"
              class="btn-icon btn-ghost sm"
              :title="KIND_TITLE[kindOf(e.name, e.is_dir)]"
              @click.stop="$emit('preview', e)"
            >
              <Icon :name="KIND_ICON[kindOf(e.name, e.is_dir)]" :size="13" />
            </button>
            <button v-if="!e.is_dir" class="btn-icon btn-ghost sm" title="下载" @click.stop="$emit('download', e)">
              <Icon name="download" :size="13" />
            </button>
            <template v-if="canWrite">
              <button class="btn-icon btn-ghost sm" title="重命名" @click.stop="$emit('rename', e)">
                <Icon name="edit" :size="13" />
              </button>
              <button class="btn-icon btn-ghost sm" title="移动到…" @click.stop="$emit('move', e)">
                <Icon name="move" :size="13" />
              </button>
              <button v-if="!e.is_dir" class="btn-icon btn-ghost sm" title="复制到…" @click.stop="$emit('copy', e)">
                <Icon name="copy" :size="13" />
              </button>
              <button class="btn-icon btn-ghost sm op-del" title="删除" @click.stop="$emit('remove', e)">
                <Icon name="trash" :size="13" />
              </button>
            </template>
          </div>
        </div>
      </div>
      </template>

      <!-- 右下角悬浮操作按钮（对齐 OpenList 官方前端交互） -->
      <div v-if="currentId && !preview" class="fab-wrap">
        <transition name="fab-pop">
          <div v-if="fabOpen" class="fab-menu">
            <button class="fab-item" @click="fabAct('mkdir')">
              <Icon name="folder-plus" :size="16" />
              <span>新建文件夹</span>
            </button>
            <button class="fab-item" @click="fabAct('upload')">
              <Icon name="upload" :size="16" />
              <span>上传文件</span>
            </button>
            <button class="fab-item" @click="fabAct('upload-folder')">
              <Icon name="upload" :size="16" />
              <span>上传文件夹</span>
            </button>
            <button class="fab-item" :disabled="refreshing" @click="fabAct('refresh')">
              <Icon name="refresh" :size="16" :class="{ spinning: refreshing }" />
              <span>刷新</span>
            </button>
          </div>
        </transition>
        <button class="fab" :class="{ open: fabOpen }" title="操作" @click="fabOpen = !fabOpen">
          <Icon name="plus" :size="22" />
        </button>
      </div>

      <!-- 隐藏的文件选择器 -->
      <input ref="fileInput" type="file" multiple hidden @change="onFilesPicked" />
      <input
        ref="folderInput"
        type="file"
        multiple
        hidden
        webkitdirectory
        directory
        @change="onFilesPicked"
      />
    </template>
  </div>
</template>

<script setup>
import { computed, ref, watch, onMounted, onBeforeUnmount } from 'vue'
import Icon from './Icon.vue'
import FileIcon from './FileIcon.vue'
import { kindOf, KIND_ICON, KIND_TITLE, TEXT_PREVIEW_MAX } from '../filekinds.js'

const props = defineProps({
  accounts: { type: Array, required: true },
  currentId: { type: String, required: true },
  crumbs: { type: Array, required: true },
  entries: { type: Array, required: true },
  preview: { type: Object, default: null },
  loading: { type: Boolean, default: false },
  refreshing: { type: Boolean, default: false },
  err: { type: String, default: '' },
  viewMode: { type: String, default: 'list' },
  driverLabels: { type: Object, default: () => ({}) },
  // 当前账号是否可写（123 分享等只读驱动隐藏写操作）
  canWrite: { type: Boolean, default: false }
})
const emit = defineEmits([
  'go-accounts', 'go-home', 'open-account', 'switch-account', 'goto', 'refresh', 'update:view-mode',
  'open-dir', 'preview', 'close-preview', 'download',
  'mkdir', 'rename', 'move', 'copy', 'remove', 'upload'
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

// 外部播放器 URL Scheme（仿 OpenList 视频页的一排播放器图标，视频/音乐共用）
const PLAYERS = [
  { name: 'PotPlayer', color: '#f5c518', url: (u) => `potplayer://${u}` },
  { name: 'VLC', color: '#f5792a', url: (u) => `vlc://${u}` },
  { name: 'MX Player', color: '#2f9cf4', url: (u) => `intent:${encodeURIComponent(u)}#Intent;package=com.mxtech.videoplayer.ad;end` },
  { name: 'IINA', color: '#7c6cf0', url: (u) => `iina://weblink?url=${encodeURIComponent(u)}` },
  { name: 'nPlayer', color: '#6e3fa3', url: (u) => `nplayer-${u}` },
  { name: 'Infuse', color: '#ff7043', url: (u) => `infuse://x-callback-url/play?url=${encodeURIComponent(u)}` }
]

function absoluteUrl() {
  try {
    return new URL(props.preview._url, window.location.href).href
  } catch {
    return props.preview._url
  }
}

function openExternal(p) {
  window.location.href = p.url(absoluteUrl())
}

function openRaw() {
  window.open(absoluteUrl(), '_blank')
}

function rowActivate(e) {
  if (e.is_dir) emit('open-dir', e)
  else if (kindOf(e.name, e.is_dir)) emit('preview', e)
}

function gridActivate(e) {
  if (e.is_drive) emit('open-account', e.id)
  else if (e.is_dir) emit('open-dir', e)
  else if (kindOf(e.name, e.is_dir)) emit('preview', e)
  else emit('download', e)
}

// ===== 同目录内预览导航（←/→ 切换上一个/下一个可预览文件） =====
const navList = computed(() => props.entries.filter((e) => !e.is_dir && kindOf(e.name, e.is_dir)))
const navIndex = computed(() =>
  props.preview ? navList.value.findIndex((e) => e.fid === props.preview.fid) : -1
)

function nav(dir) {
  const i = navIndex.value + dir
  const t = navList.value[i]
  if (t) emit('preview', t)
}

function onKey(ev) {
  if (!props.preview) return
  if (ev.key === 'Escape') emit('close-preview')
  else if (ev.key === 'ArrowLeft') nav(-1)
  else if (ev.key === 'ArrowRight') nav(1)
}
onMounted(() => window.addEventListener('keydown', onKey))
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))

// ===== 文本类预览：fetch 全文展示（server_proxy 关闭时为跨域直链，失败则降级提示） =====
const textContent = ref(null)
const textLoading = ref(false)
const textError = ref('')
const audioPlaying = ref(false)

watch(
  () => props.preview,
  (p) => {
    audioPlaying.value = false
    textContent.value = null
    textError.value = ''
    if (!p || p._kind !== 'text') return
    if (p.size != null && p.size > TEXT_PREVIEW_MAX) {
      textError.value = '文件过大，不支持在线预览'
      return
    }
    textLoading.value = true
    fetch(p._url)
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`)
        return r.text()
      })
      .then((t) => {
        textContent.value = t
      })
      .catch(() => {
        textError.value = '无法加载文本内容（直链跨域或网络失败）'
      })
      .finally(() => {
        textLoading.value = false
      })
  }
)

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

// ===== 右下角 FAB 操作菜单 =====
const fabOpen = ref(false)
const fileInput = ref(null)
const folderInput = ref(null)
let pickMode = 'files'

function fabAct(act) {
  fabOpen.value = false
  if (act === 'mkdir') emit('mkdir')
  else if (act === 'refresh') emit('refresh')
  else if (act === 'upload') {
    pickMode = 'files'
    fileInput.value?.click()
  } else if (act === 'upload-folder') {
    pickMode = 'folder'
    folderInput.value?.click()
  }
}

function onFilesPicked(ev) {
  const files = Array.from(ev.target.files || [])
  ev.target.value = ''
  if (files.length === 0) return
  const isFolder = pickMode === 'folder'
  emit(
    'upload',
    files.map((f) => ({
      file: f,
      // 文件夹上传时保留相对路径（含中间目录），单文件只用文件名
      relPath: isFolder ? (f.webkitRelativePath || f.name) : f.name
    }))
  )
}

// 点击 FAB 菜单外部时收起
function onDocClick(ev) {
  if (!fabOpen.value) return
  if (ev.target.closest?.('.fab-wrap')) return
  fabOpen.value = false
}
onMounted(() => document.addEventListener('click', onDocClick))
onBeforeUnmount(() => document.removeEventListener('click', onDocClick))
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
.op-del:hover {
  color: #e5484d;
}

/* 右下角 FAB 操作菜单（对齐 OpenList 官方前端） */
.fab-wrap {
  position: fixed;
  right: 28px;
  bottom: 28px;
  z-index: 80;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 10px;
}
.fab {
  width: 52px;
  height: 52px;
  border-radius: 50%;
  border: none;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  background: linear-gradient(135deg, #5b7dff, #7c4dff);
  box-shadow: 0 8px 24px rgba(91, 125, 255, 0.4);
  transition: transform 0.2s ease, box-shadow 0.2s ease;
}
.fab:hover {
  transform: scale(1.06);
  box-shadow: 0 10px 28px rgba(91, 125, 255, 0.5);
}
.fab.open {
  transform: rotate(45deg);
}
.fab-menu {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 2px;
  padding: 6px;
  background: var(--ol-panel);
  border: 1px solid var(--ol-border);
  border-radius: var(--ol-radius-sm, 10px);
  box-shadow: var(--ol-shadow, 0 8px 30px rgba(0, 0, 0, 0.12));
  min-width: 148px;
}
.fab-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 12px;
  border: none;
  background: transparent;
  border-radius: 8px;
  color: var(--ol-text);
  font-size: 13.5px;
  cursor: pointer;
  white-space: nowrap;
}
.fab-item:hover {
  background: var(--ol-primary-light);
  color: var(--ol-primary);
}
.fab-item:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.fab-pop-enter-active,
.fab-pop-leave-active {
  transition: opacity 0.15s ease, transform 0.15s ease;
}
.fab-pop-enter-from,
.fab-pop-leave-to {
  opacity: 0;
  transform: translateY(8px) scale(0.96);
}
.refresh-btn {
  color: var(--ol-text-dim);
}
.refresh-btn:hover {
  color: var(--ol-primary);
}
.refresh-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
.refresh-btn.spinning svg {
  animation: refresh-spin 0.8s linear infinite;
}
@keyframes refresh-spin {
  to {
    transform: rotate(360deg);
  }
}
.fab-item .spinning {
  animation: refresh-spin 0.8s linear infinite;
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
  gap: 8px;
  color: var(--ol-text);
  max-width: 100%;
  overflow: hidden;
}
.fname-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  word-break: normal;
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

/* 统一预览卡片（视频/音乐/图片/PDF/文本） */
.player-card {
  padding: 0;
  overflow: hidden;
}
.player-head {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-bottom: 1px solid var(--ol-border);
}
.player-kind {
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 8px;
  background: var(--ol-primary-light);
  color: var(--ol-primary);
}
.player-fname {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  color: var(--ol-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.player-pos {
  flex-shrink: 0;
  font-size: 12px;
  color: var(--ol-text-faint);
  font-variant-numeric: tabular-nums;
}
.player-nav {
  display: flex;
  gap: 2px;
  flex-shrink: 0;
}
.player-nav button:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}

.player-video {
  display: block;
  width: 100%;
  aspect-ratio: 16 / 9;
  background: #000;
  outline: none;
}

/* 音乐 */
.audio-stage {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 22px;
  padding: 40px 20px 34px;
  background: linear-gradient(165deg, rgba(91, 125, 255, 0.1), rgba(124, 77, 255, 0.1));
}
.disc {
  width: 116px;
  height: 116px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  background:
    radial-gradient(circle at center, rgba(255, 255, 255, 0.9) 0 11%, transparent 11.5%),
    linear-gradient(135deg, #5b7dff, #7c4dff);
  box-shadow: 0 10px 30px rgba(91, 125, 255, 0.35), inset 0 0 0 6px rgba(255, 255, 255, 0.12);
}
.disc.spinning {
  animation: disc-spin 5s linear infinite;
}
@keyframes disc-spin {
  to {
    transform: rotate(360deg);
  }
}
.audio-player {
  width: min(520px, 100%);
  outline: none;
}

/* 图片 */
.image-stage {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 200px;
  background: #14161a;
  background-image:
    linear-gradient(45deg, #1c1f26 25%, transparent 25%, transparent 75%, #1c1f26 75%),
    linear-gradient(45deg, #1c1f26 25%, transparent 25%, transparent 75%, #1c1f26 75%);
  background-size: 24px 24px;
  background-position: 0 0, 12px 12px;
}
.image-view {
  display: block;
  max-width: 100%;
  max-height: 68vh;
  object-fit: contain;
}

/* PDF */
.pdf-frame {
  display: block;
  width: 100%;
  height: 72vh;
  border: none;
  background: #525659;
}

/* 文本 */
.text-stage {
  background: var(--ol-bg);
}
.text-view {
  margin: 0;
  padding: 16px 18px;
  max-height: 68vh;
  overflow: auto;
  font-family: var(--ol-mono, ui-monospace, Consolas, monospace);
  font-size: 13px;
  line-height: 1.65;
  color: var(--ol-text);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  tab-size: 4;
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

/* 移动端卡片列表：默认隐藏，移动端显示 */
.mobile-list {
  display: none;
  list-style: none;
  margin: 0;
  padding: 0;
}
.mobile-item {
  display: flex;
  flex-direction: column;
  padding: 10px 14px;
  border-bottom: 1px solid var(--ol-border);
  gap: 4px;
}
.mobile-item:last-child {
  border-bottom: none;
}
.mobile-item.drive-row {
  cursor: pointer;
}
.mobile-item.drive-row:active {
  background: var(--ol-primary-light);
}
/* 第一行：图标 + 文件名 + 大小/时间 */
.mobile-row-info {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.mobile-fname {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--ol-text);
  font-size: 14px;
}
a.mobile-fname {
  color: var(--ol-text);
}
a.mobile-fname:hover {
  color: var(--ol-primary);
  text-decoration: none;
}
.mobile-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
  font-size: 12px;
  color: var(--ol-text-dim);
  white-space: nowrap;
}
.mobile-date {
  color: var(--ol-text-faint);
}
/* 第二行：操作按钮右对齐 */
.mobile-row-ops {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 2px;
  padding-left: 28px; /* 与图标宽度对齐，视觉缩进 */
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
  .player-pos {
    display: none;
  }
  /* 切换：隐藏桌面表格，显示移动端卡片 */
  .desktop-table {
    display: none;
  }
  .mobile-list {
    display: block;
  }
  .fab-wrap {
    right: 16px;
    bottom: 20px;
  }
}
</style>
