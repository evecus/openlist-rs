<template>
  <div class="page">
    <div v-if="accounts.length === 0" class="empty-state card">
      <Icon name="inbox" :size="40" />
      <p>还没有存储，先去 <a href="#" @click.prevent="$emit('go-accounts')">存储管理</a> 添加一个云盘账号</p>
    </div>
    <template v-else>
      <div class="crumbs-row">
        <nav class="crumbs">
          <a href="#" class="crumb-link crumb-home" title="返回首页" @click.prevent="$emit('go-home')">🏠 首页</a>
          <template v-if="currentId">
            <template v-for="(c, i) in crumbs" :key="c.fid + i">
              <span class="crumb-sep">/</span>
              <a v-if="preview || i !== crumbs.length - 1" href="#" class="crumb-link" @click.prevent="$emit('goto', i)">{{ c.name }}</a>
              <span v-else class="crumb-link crumb-current">{{ c.name }}</span>
            </template>
            <template v-if="preview">
              <span class="crumb-sep">/</span>
              <span class="crumb-link crumb-current">{{ preview.name }}</span>
            </template>
          </template>
        </nav>
        <div v-if="!preview" class="view-toggle" role="group" aria-label="视图切换">
          <button :class="{ active: viewMode === 'list' }" :title="viewMode === 'list' ? '切换为宫格' : '列表'" @click="$emit('update:view-mode', viewMode === 'list' ? 'grid' : 'list')">
            <Icon :name="viewMode === 'list' ? 'grid' : 'list'" :size="16" />
          </button>
        </div>
      </div>
      <div v-if="err" class="alert alert-error">{{ err }}</div>
      <template v-if="preview">
        <div class="file-line">
          <span class="file-line-name" :title="preview.name">{{ preview.name }}</span>
          <span v-if="navList.length > 1" class="player-pos">{{ navIndex + 1 }} / {{ navList.length }}</span>
          <span class="file-line-ops">
            <button class="op-icon" :disabled="navIndex <= 0" title="上一个" @click="nav(-1)"><Icon name="chevron-left" :size="15" /></button>
            <button class="op-icon" :disabled="navIndex >= navList.length - 1" title="下一个" @click="nav(1)"><Icon name="chevron-right" :size="15" /></button>
            <button class="op-icon" title="下载" @click="$emit('download', preview)"><Icon name="download" :size="15" /></button>
            <button class="op-icon" title="关闭" @click="$emit('close-preview')"><Icon name="close" :size="15" /></button>
          </span>
        </div>
        <div class="obj-box">
          <div v-if="preview._kind === 'video'" class="video-stage"><video ref="videoEl" :src="preview._url" controls autoplay playsinline class="player-video" @ended="onVideoEnded"></video></div>
          <div v-else-if="preview._kind === 'audio'" class="audio-stage"><div class="disc" :class="{ spinning: audioPlaying }"><Icon name="music" :size="44" /></div><audio :src="preview._url" controls autoplay class="audio-player" @play="audioPlaying = true" @pause="audioPlaying = false" @ended="audioPlaying = false"></audio></div>
          <div v-else-if="preview._kind === 'image'" class="image-stage"><img :src="preview._url" :alt="preview.name" class="image-view" /></div>
          <iframe v-else-if="preview._kind === 'pdf'" :src="preview._url" class="pdf-frame" title="PDF"></iframe>
          <div v-else-if="preview._kind === 'text'" class="text-stage">
            <div v-if="textLoading" class="state-box"><span class="spin loader-lg"></span><span>加载中…</span></div>
            <div v-else-if="textError" class="state-box"><Icon name="alert" :size="30" /><span>{{ textError }}</span></div>
            <div v-else class="text-editor"><div class="text-gutter" aria-hidden="true"><span v-for="n in textLineCount" :key="n">{{ n }}</span></div><pre class="text-view">{{ textContent }}</pre></div>
          </div>
        </div>
      </template>
      <template v-else>
        <div v-if="currentId && loading" class="obj-box state-box"><span class="spin loader-lg"></span><span>加载中…</span></div>
        <div v-else-if="currentId && entries.length === 0" class="obj-box state-box"><Icon name="inbox" :size="34" /><span>此文件夹为空</span></div>
        <div v-else-if="viewMode === 'list'" class="obj-box">
          <div class="list-title"><span class="lt-name">名称</span><span class="lt-size">大小</span><span class="lt-date">修改时间</span><span class="lt-op"></span></div>
          <div class="list-body desktop-list">
            <div v-for="e in displayEntries" :key="e.key" class="list-item" @click="rowActivate(e)">
              <span class="li-name"><span class="ficon"><FileIcon :name="e.name" :is-dir="e.is_dir" /></span><span class="fname-text">{{ e.name }}</span></span>
              <span class="li-size">{{ e.is_dir || e.is_drive ? '-' : fmtSize(e.size) }}</span>
              <span class="li-date">{{ e.updated_at ? fmtDate(e.updated_at) : '-' }}</span>
              <span class="li-op">
                <template v-if="!e.is_drive">
                  <button v-if="kindOf(e.name, e.is_dir)" class="op-icon" :title="KIND_TITLE[kindOf(e.name, e.is_dir)]" @click.stop="$emit('preview', e)"><Icon :name="KIND_ICON[kindOf(e.name, e.is_dir)]" :size="14" /></button>
                  <button v-if="!e.is_dir" class="op-icon" title="下载" @click.stop="$emit('download', e)"><Icon name="download" :size="14" /></button>
                  <template v-if="canWrite">
                    <button class="op-icon" title="重命名" @click.stop="$emit('rename', e)"><Icon name="edit" :size="14" /></button>
                    <button class="op-icon" title="移动到…" @click.stop="$emit('move', e)"><Icon name="move" :size="14" /></button>
                    <button v-if="!e.is_dir" class="op-icon" title="复制到…" @click.stop="$emit('copy', e)"><Icon name="copy" :size="14" /></button>
                    <button class="op-icon op-del" title="删除" @click.stop="$emit('remove', e)"><Icon name="trash" :size="14" /></button>
                  </template>
                </template>
              </span>
            </div>
          </div>
          <ul class="list-body mobile-list">
            <li v-for="e in displayEntries" :key="e.key" class="list-item mobile-item" @click="e.is_drive ? $emit('open-account', e.id) : rowActivate(e)">
              <div class="mobile-row-info">
                <span class="ficon"><FileIcon :name="e.name" :is-dir="e.is_dir" /></span>
                <span class="fname-text">{{ e.name }}</span>
              </div>
              <div class="mobile-meta">
                <span>{{ e.is_dir || e.is_drive ? '-' : fmtSize(e.size) }}</span>
                <span v-if="e.updated_at" class="mobile-date">{{ fmtDate(e.updated_at) }}</span>
              </div>
              <div v-if="!e.is_drive" class="mobile-row-ops">
                <button v-if="kindOf(e.name, e.is_dir)" class="op-icon" :title="KIND_TITLE[kindOf(e.name, e.is_dir)]" @click.stop="$emit('preview', e)"><Icon :name="KIND_ICON[kindOf(e.name, e.is_dir)]" :size="14" /></button>
                <button v-if="!e.is_dir" class="op-icon" title="下载" @click.stop="$emit('download', e)"><Icon name="download" :size="14" /></button>
                <template v-if="canWrite">
                  <button class="op-icon" title="重命名" @click.stop="$emit('rename', e)"><Icon name="edit" :size="14" /></button>
                  <button class="op-icon" title="移动到…" @click.stop="$emit('move', e)"><Icon name="move" :size="14" /></button>
                  <button v-if="!e.is_dir" class="op-icon" title="复制到…" @click.stop="$emit('copy', e)"><Icon name="copy" :size="14" /></button>
                  <button class="op-icon op-del" title="删除" @click.stop="$emit('remove', e)"><Icon name="trash" :size="14" /></button>
                </template>
              </div>
            </li>
          </ul>
        </div>
        <div v-else class="obj-box">
          <div class="grid-view">
            <div v-for="e in displayEntries" :key="e.key" class="grid-item" @click="gridActivate(e)">
              <div class="grid-icon"><FileIcon :name="e.name" :is-dir="e.is_dir" /></div>
              <div class="grid-name" :title="e.name">{{ e.name }}</div>
            </div>
          </div>
        </div>
      </template>
      <div v-if="currentId && !preview" class="fab-wrap">
        <transition name="fab-pop">
          <div v-if="fabOpen" class="fab-menu">
            <button class="fab-item" :disabled="refreshing" title="刷新" @click="fabAct('refresh')"><Icon name="refresh" :size="17" :class="{ spinning: refreshing }" /></button>
            <button class="fab-item" title="新建文件夹" @click="fabAct('mkdir')"><Icon name="folder-plus" :size="17" /></button>
            <button class="fab-item" title="上传文件" @click="fabAct('upload')"><Icon name="upload" :size="17" /></button>
          </div>
        </transition>
        <button class="fab" :class="{ open: fabOpen }" title="操作" @click="fabOpen = !fabOpen"><Icon name="more" :size="22" /></button>
      </div>
      <input ref="fileInput" type="file" multiple hidden @change="onFilesPicked" />
      <input ref="folderInput" type="file" multiple hidden webkitdirectory directory @change="onFilesPicked" />
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
  canWrite: { type: Boolean, default: false }
})
const emit = defineEmits([
  'go-accounts', 'go-home', 'open-account', 'switch-account', 'goto', 'refresh', 'update:view-mode',
  'open-dir', 'preview', 'close-preview', 'download',
  'mkdir', 'rename', 'move', 'copy', 'remove', 'upload'
])

const displayEntries = computed(() => {
  if (props.currentId) return props.entries.map((e) => ({ ...e, key: e.fid }))
  return props.accounts.filter((a) => a.enabled !== false).map((a) => ({
    key: 'drive-' + a.id, id: a.id, name: a.name, is_dir: true, is_drive: true,
    driver: a.driver, size: null, updated_at: null
  }))
})

const fabOpen = ref(false)
const fileInput = ref(null)
const folderInput = ref(null)
const videoEl = ref(null)
const audioPlaying = ref(false)
const textContent = ref('')
const textLoading = ref(false)
const textError = ref('')
const autoNext = ref(true)

const navList = computed(() => {
  if (!props.preview) return []
  return props.entries.filter((e) => !e.is_dir && kindOf(e.name, false))
})
const navIndex = computed(() => navList.value.findIndex((e) => e.fid === props.preview?.fid))
const textLineCount = computed(() => Math.max(1, (textContent.value || '').split('\n').length))

function fmtSize(n) {
  if (n == null || n < 0) return '-'
  const u = ['B', 'KB', 'MB', 'GB', 'TB']
  let i = 0, v = Number(n)
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++ }
  return (i === 0 ? v : v.toFixed(v < 10 ? 1 : 0)) + ' ' + u[i]
}
function fmtDate(s) {
  if (!s) return '-'
  try {
    const d = new Date(s)
    const p = (n) => String(n).padStart(2, '0')
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
  } catch { return s }
}
function rowActivate(e) {
  if (e.is_drive) emit('open-account', e.id)
  else if (e.is_dir) emit('open-dir', e)
  else if (kindOf(e.name, false)) emit('preview', e)
  else emit('download', e)
}
function gridActivate(e) { rowActivate(e) }
function nav(dir) {
  const i = navIndex.value + dir
  if (i >= 0 && i < navList.value.length) emit('preview', navList.value[i])
}
function onVideoEnded() { if (autoNext.value) nav(1) }
function fabAct(act) {
  fabOpen.value = false
  if (act === 'refresh') emit('refresh')
  else if (act === 'mkdir') emit('mkdir')
  else if (act === 'upload') fileInput.value?.click()
  else if (act === 'upload-folder') folderInput.value?.click()
}
function onFilesPicked(ev) {
  const files = Array.from(ev.target.files || [])
  if (files.length) emit('upload', files)
  ev.target.value = ''
}

watch(() => props.preview, async (p) => {
  textContent.value = ''; textError.value = ''; textLoading.value = false
  if (!p || p._kind !== 'text' || !p._url) return
  textLoading.value = true
  try {
    const r = await fetch(p._url)
    if (!r.ok) throw new Error('HTTP ' + r.status)
    const t = await r.text()
    textContent.value = t.length > TEXT_PREVIEW_MAX ? t.slice(0, TEXT_PREVIEW_MAX) + '\n…(截断)' : t
  } catch (e) { textError.value = String(e.message || e) }
  finally { textLoading.value = false }
}, { immediate: true })

function onKey(e) {
  if (!props.preview) return
  if (e.key === 'Escape') emit('close-preview')
  else if (e.key === 'ArrowLeft') nav(-1)
  else if (e.key === 'ArrowRight') nav(1)
}
onMounted(() => window.addEventListener('keydown', onKey))
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))
</script>

<style scoped>
.page { max-width: 980px; margin: 0 auto; padding: 4px 20px 40px; }
.empty-state { display: flex; flex-direction: column; align-items: center; gap: 12px; color: var(--ol-text-dim); padding: 60px 20px; text-align: center; }
.crumbs-row { display: flex; align-items: center; gap: 8px; }
.crumbs { display: flex; align-items: center; flex-wrap: wrap; gap: 2px; padding: 14px 0; font-size: 14px; flex: 1; min-width: 0; }
.crumb-link { color: var(--ol-text); padding: 4px 10px; border-radius: 8px; word-break: break-all; }
.crumb-link:hover { background: rgba(0,0,0,0.05); text-decoration: none; }
.crumb-sep { color: var(--ol-text-faint); padding: 0 2px; }
.view-toggle button { width: 34px; height: 34px; display: flex; align-items: center; justify-content: center; border: none; border-radius: 9px; background: var(--ol-accent-light); color: var(--ol-accent); cursor: pointer; flex-shrink: 0; }
.file-line { display: flex; align-items: center; gap: 10px; padding: 2px 4px 12px; font-size: 14px; }
.file-line-name { flex: 1; min-width: 0; word-break: break-all; color: var(--ol-text); }
.player-pos { flex-shrink: 0; font-size: 12px; color: var(--ol-text-faint); }
.file-line-ops { display: flex; gap: 2px; flex-shrink: 0; }
.obj-box { background: var(--ol-panel); border-radius: 12px; box-shadow: 0 8px 24px rgba(0,0,0,0.06); padding: 8px; }
.list-title { display: flex; align-items: center; padding: 8px; font-size: 13px; font-weight: 700; color: var(--ol-text-dim); }
.lt-name { width: 50%; } .lt-size { width: 17%; text-align: right; } .lt-date { width: 33%; text-align: right; } .lt-op { width: 132px; flex-shrink: 0; }
.list-body { display: flex; flex-direction: column; gap: 2px; list-style: none; margin: 0; padding: 0; }
.list-item { display: flex; align-items: center; width: 100%; padding: 8px; border-radius: 10px; cursor: pointer; transition: all 0.3s; }
.list-item:hover { transform: scale(1.01); background: rgba(0,0,0,0.045); }
.li-name { display: flex; align-items: center; gap: 8px; width: 50%; min-width: 0; padding-right: 8px; }
.fname-text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--ol-text); }
.li-size { width: 17%; text-align: right; color: var(--ol-text-dim); font-size: 13px; white-space: nowrap; }
.li-date { width: 33%; text-align: right; color: var(--ol-text-dim); font-size: 13px; white-space: nowrap; }
.li-op { width: 132px; flex-shrink: 0; display: flex; justify-content: flex-end; gap: 2px; opacity: 0; transition: opacity 0.15s ease; }
.list-item:hover .li-op { opacity: 1; }
.ficon { width: 24px; height: 24px; flex-shrink: 0; display: flex; align-items: center; }
.op-icon { width: 28px; height: 28px; display: inline-flex; align-items: center; justify-content: center; border: none; border-radius: 7px; background: transparent; color: var(--ol-text-dim); cursor: pointer; flex-shrink: 0; }
.op-icon:hover { background: rgba(0,0,0,0.06); color: var(--ol-text); }
.op-del:hover { color: var(--ol-danger); }
.state-box { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; padding: 60px 0; color: var(--ol-text-dim); font-size: 13.5px; }
.loader-lg { width: 26px; height: 26px; border: 3px solid var(--ol-border-strong); border-top-color: var(--ol-primary); border-radius: 50%; }
.video-stage { background: #000; border-radius: 10px; overflow: hidden; }
.player-video { display: block; width: 100%; max-height: 60vh; aspect-ratio: 16/9; background: #000; outline: none; }
.audio-stage { display: flex; flex-direction: column; align-items: center; gap: 16px; padding: 40px 20px; }
.disc { width: 96px; height: 96px; border-radius: 50%; background: var(--ol-primary-light); display: flex; align-items: center; justify-content: center; color: var(--ol-primary); }
.disc.spinning { animation: ol-spin 3s linear infinite; }
.image-stage { display: flex; justify-content: center; padding: 12px; }
.image-view { max-width: 100%; max-height: 70vh; border-radius: 8px; }
.pdf-frame { width: 100%; height: 70vh; border: none; border-radius: 8px; }
.text-stage { min-height: 200px; }
.text-editor { display: flex; max-height: 68vh; overflow: auto; background: var(--ol-bg); border-radius: 10px; }
.text-gutter { flex-shrink: 0; min-width: 44px; padding: 16px 8px 16px 12px; text-align: right; font-family: ui-monospace, Consolas, monospace; font-size: 12px; line-height: 1.65; color: var(--ol-text-faint); user-select: none; border-right: 1px solid var(--ol-border); }
.text-gutter span { display: block; }
.text-view { margin: 0; flex: 1; padding: 16px 18px; font-family: ui-monospace, Consolas, monospace; font-size: 13px; line-height: 1.65; color: var(--ol-text); white-space: pre; }
.grid-view { display: grid; grid-template-columns: repeat(auto-fill, minmax(110px, 1fr)); gap: 8px; padding: 8px; }
.grid-item { display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 12px 8px; border-radius: 10px; cursor: pointer; transition: background 0.15s; }
.grid-item:hover { background: rgba(0,0,0,0.04); }
.grid-icon { width: 48px; height: 48px; }
.grid-name { font-size: 12px; text-align: center; word-break: break-all; color: var(--ol-text); max-width: 100%; overflow: hidden; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; }
.fab-wrap { position: fixed; right: 20px; bottom: 28px; z-index: 50; display: flex; flex-direction: column; align-items: center; gap: 8px; }
.fab { width: 52px; height: 52px; border-radius: 50%; border: none; display: flex; align-items: center; justify-content: center; color: var(--ol-text); background: var(--ol-panel); box-shadow: 0 4px 16px rgba(0,0,0,0.18); cursor: pointer; }
.fab.open { color: var(--ol-primary); }
.fab-menu { display: flex; flex-direction: column; align-items: center; gap: 2px; padding: 4px; background: var(--ol-panel); border-radius: 12px; box-shadow: 0 8px 28px rgba(0,0,0,0.16); }
.fab-item { width: 40px; height: 40px; display: flex; align-items: center; justify-content: center; border: none; background: transparent; border-radius: 50%; color: var(--ol-text); cursor: pointer; }
.fab-item:hover { background: rgba(0,0,0,0.06); color: var(--ol-primary); }
.fab-pop-enter-active, .fab-pop-leave-active { transition: opacity 0.15s ease, transform 0.15s ease; }
.fab-pop-enter-from, .fab-pop-leave-to { opacity: 0; transform: translateY(8px) scale(0.96); }
.spinning { animation: ol-spin 0.8s linear infinite; }
@keyframes ol-spin { to { transform: rotate(360deg); } }
.mobile-row-info { display: flex; align-items: center; gap: 8px; min-width: 0; flex: 1; }
.mobile-meta { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--ol-text-dim); white-space: nowrap; }
.mobile-date { color: var(--ol-text-faint); }
.mobile-row-ops { display: flex; align-items: center; justify-content: flex-end; gap: 2px; padding-left: 32px; }
.mobile-row-ops .op-icon { opacity: 1; }
@media (max-width: 768px) {
  .page { padding: 4px 12px 40px; }
  .lt-date, .li-date { display: none; }
  .lt-name, .li-name { width: 76%; }
  .lt-size, .li-size { width: 24%; }
  .lt-op, .li-op { display: none; }
  .desktop-list { display: none; }
  .mobile-list { display: flex; }
  .mobile-item { flex-wrap: wrap; align-items: flex-start; }
  .mobile-item .mobile-row-info { flex: 1 1 100%; width: 100%; max-width: 100%; }
  .mobile-item .mobile-meta { flex: 1 1 100%; width: 100%; max-width: 100%; padding-left: 32px; order: 2; }
  .mobile-item .mobile-row-ops { flex: 1 1 100%; width: 100%; max-width: 100%; order: 3; }
}
@media (min-width: 769px) {
  .mobile-list { display: none; }
}
</style>
