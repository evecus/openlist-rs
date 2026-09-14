<template>
  <LoginView
    v-if="needLogin"
    :form="loginForm"
    :error="loginError"
    :loading="loggingIn"
    @update:form="(v) => (loginForm = v)"
    @login="doLogin"
  />

<div class="app-shell">
    <header class="app-header">
      <div class="header-inner">
        <div class="brand" @click="goHome">
          <span class="brand-icon">
            <svg viewBox="0 0 48 48" fill="none">
              <path
                d="M6 12a2 2 0 0 1 2-2h10.5l3 4H40a2 2 0 0 1 2 2v20a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V12Z"
                fill="#fff"
                fill-opacity="0.92"
              />
              <path d="M6 16h36v16a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V16Z" fill="#fff" />
            </svg>
          </span>
          <span class="brand-name">OpenList</span>
        </div>

        <nav class="app-nav">
          <button class="nav-btn" :class="{ active: view === 'files' }" @click="view = 'files'">
            文件
          </button>
          <button class="nav-btn" :class="{ active: view === 'accounts' }" @click="view = 'accounts'">
            存储管理
          </button>
        </nav>

        <div class="header-actions">
          <button class="btn-icon btn-ghost" title="切换主题" @click="toggleTheme">
            <Icon :name="isDark ? 'sun' : 'moon'" :size="17" />
          </button>
          <button v-if="authEnabled" class="btn-icon btn-ghost" title="退出登录" @click="doLogout">
            <Icon name="logout" :size="17" />
          </button>
        </div>
      </div>
    </header>

    <AccountsView
      v-if="view === 'accounts'"
      :accounts="accounts"
      :error="accError"
      :adding="adding"
      :driver-labels="DRIVER_LABELS"
      :fetch-secret="fetchAccountSecret"
      @add="addAccount"
      @edit="editAccount"
      @delete="delAccount"
    />

    <FilesView
      v-else
      :accounts="accounts"
      :current-id="currentId"
      :crumbs="crumbs"
      :entries="entries"
      :playing="playing"
      :loading="loading"
      :refreshing="refreshing"
      :err="err"
      :driver-labels="DRIVER_LABELS"
      v-model:view-mode="viewMode"
      @go-accounts="view = 'accounts'"
      @go-home="goHome"
      @open-account="onSwitchAccount"
      @switch-account="onSwitchAccount"
      @goto="goto"
      @refresh="refreshCurrent"
      @open-dir="openDir"
      @play="play"
      @download="download"
    />
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import Icon from './components/Icon.vue'
import LoginView from './components/LoginView.vue'
import AccountsView from './components/AccountsView.vue'
import FilesView from './components/FilesView.vue'

const view = ref('files')
const viewMode = ref('list')
const accounts = ref([])
const currentId = ref('')
const entries = ref([])
const crumbs = ref([{ fid: '0', name: '根目录' }])
const loading = ref(false)
const refreshing = ref(false)
const err = ref('')

const accError = ref('')
const adding = ref(false)

const DRIVER_LABELS = {
  quark: '夸克',
  quark_uc: 'UC',
  '123pan': '123',
  aliyundrive_open: '阿里',
  baidu_netdisk: '百度',
  pan115: '115',
  thunder: '迅雷',
  lanzou: '蓝奏云'
}

// 鉴权
const needLogin = ref(false)
const authEnabled = ref(false)
const loginForm = ref({ username: '', password: '' })
const loginError = ref('')
const loggingIn = ref(false)

// 播放器
const playing = ref(null)

// 主题
const isDark = ref(false)
function applyTheme() {
  document.documentElement.classList.toggle('dark', isDark.value)
}
function toggleTheme() {
  isDark.value = !isDark.value
  localStorage.setItem('ol-theme', isDark.value ? 'dark' : 'light')
  applyTheme()
}

async function api(path, opts) {
  const r = await fetch(path, opts)
  let body = {}
  try {
    body = await r.json()
  } catch {
    /* 非 JSON */
  }
  if ((r.status === 401 || body.code === 401) && authEnabled.value) {
    needLogin.value = true
    throw new Error('未登录或会话已过期')
  }
  if (!r.ok) throw new Error(body.message || body.error || r.statusText || `HTTP ${r.status}`)
  return body
}

async function checkAuth() {
  const b = await fetch('/api/auth/status')
    .then((r) => r.json())
    .catch(() => ({ enabled: false }))
  authEnabled.value = !!b.enabled
}

async function doLogin() {
  loginError.value = ''
  loggingIn.value = true
  try {
    const r = await fetch('/api/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(loginForm.value)
    })
    if (!r.ok) {
      const b = await r.json().catch(() => ({}))
      throw new Error(b.error || '登录失败')
    }
    needLogin.value = false
    loginForm.value = { username: '', password: '' }
    await loadAccounts()
  } catch (e) {
    loginError.value = e.message
  } finally {
    loggingIn.value = false
  }
}

async function doLogout() {
  await fetch('/api/logout', { method: 'POST' }).catch(() => {})
  needLogin.value = true
}

async function fetchAccountSecret(id) {
  return api(`/api/accounts/${id}/secret`)
}

async function loadAccounts() {
  const b = await api('/api/accounts')
  accounts.value = b.accounts || []
}

async function addAccount(form) {
  accError.value = ''
  adding.value = true
  try {
    const payload = { name: form.name, driver: form.driver, server_proxy: form.server_proxy ?? false }
    const d = form.driver
    if (['quark', 'quark_uc', 'pan115', 'lanzou'].includes(d)) payload.cookie = form.cookie
    else if (['123pan', 'thunder'].includes(d)) {
      payload.username = form.username
      payload.password = form.password
    } else if (['aliyundrive_open', 'baidu_netdisk'].includes(d)) payload.refresh_token = form.refresh_token

    await api('/api/accounts', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload)
    })
    await loadAccounts()
  } catch (e) {
    accError.value = e.message
  } finally {
    adding.value = false
  }
}

async function editAccount(form) {
  accError.value = ''
  adding.value = true
  try {
    const payload = { name: form.name, driver: form.driver, server_proxy: form.server_proxy ?? false }
    const d = form.driver
    if (['quark', 'quark_uc', 'pan115', 'lanzou'].includes(d)) payload.cookie = form.cookie
    else if (['123pan', 'thunder'].includes(d)) {
      payload.username = form.username
      payload.password = form.password
    } else if (['aliyundrive_open', 'baidu_netdisk'].includes(d)) payload.refresh_token = form.refresh_token

    await api(`/api/accounts/${form.id}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload)
    })
    await loadAccounts()
  } catch (e) {
    accError.value = e.message
  } finally {
    adding.value = false
  }
}

async function delAccount(a) {
  if (!confirm(`删除存储「${a.name}」？`)) return
  await api(`/api/accounts/${a.id}`, { method: 'DELETE' })
  if (currentId.value === a.id) {
    currentId.value = ''
    entries.value = []
    crumbs.value = [{ fid: '0', name: '根目录' }]
  }
  await loadAccounts()
}

function onSwitchAccount(id) {
  view.value = 'files'
  currentId.value = id
  const acc = accounts.value.find((a) => a.id === id)
  crumbs.value = [{ fid: '0', name: acc?.name || '根目录' }]
  listFiles('0')
}

// 返回网盘列表首页：不自动加载任何网盘的内容
function goHome() {
  playing.value = null
  view.value = 'files'
  currentId.value = ''
  entries.value = []
  crumbs.value = [{ fid: '0', name: '网盘' }]
}

// 刷新：强制向网盘重新拉取当前目录（跳过服务端缓存），刷新按钮转圈反馈
async function refreshCurrent() {
  if (!currentId.value) {
    await loadAccounts()
    return
  }
  if (refreshing.value) return
  refreshing.value = true
  try {
    await listFiles(crumbs.value[crumbs.value.length - 1].fid, true)
  } finally {
    refreshing.value = false
  }
}

// refresh=true 时请求跳过服务端缓存；此时不清空旧列表、不显示整页 loading，
// 由刷新按钮转圈反馈，失败时保留旧数据只提示错误
async function listFiles(fid, refresh = false) {
  if (!currentId.value) return
  if (!refresh) loading.value = true
  err.value = ''
  try {
    const b = await api(
      `/api/files?account=${encodeURIComponent(currentId.value)}&fid=${encodeURIComponent(fid)}${refresh ? '&refresh=true' : ''}`
    )
    entries.value = b.entries || []
  } catch (e) {
    err.value = e.message
    if (!refresh) entries.value = []
  } finally {
    loading.value = false
  }
}

function openDir(e) {
  crumbs.value.push({ fid: e.fid, name: e.name })
  listFiles(e.fid)
}

function goto(i) {
  playing.value = null
  crumbs.value = crumbs.value.slice(0, i + 1)
  listFiles(crumbs.value[i].fid)
}

function fileQuery(e, disp) {
  const p = new URLSearchParams({ account: currentId.value, fid: e.fid })
  if (e.name) p.set('name', e.name)
  if (e.size) p.set('size', e.size)
  if (e.etag) p.set('etag', e.etag)
  if (e.s3_key_flag) p.set('s3key', e.s3_key_flag)
  if (e.file_type != null) p.set('ftype', e.file_type)
  if (e.extra) p.set('extra', JSON.stringify(e.extra))
  if (disp) p.set('disp', disp)
  return p.toString()
}

// 取真实播放/下载地址：由后端根据账号 server_proxy 开关决定
// 返回网盘直链（开关关闭）还是本服务 /api/stream 中转地址（开关开启）
async function resolveUrl(e) {
  const b = await api(`/api/download?${fileQuery(e)}`)
  return b.url
}

function download(e) {
  resolveUrl(e)
    .then((url) => window.open(url, '_blank'))
    .catch((err) => {
      accError.value = err.message || '获取下载地址失败'
    })
}

async function play(e) {
  try {
    const url = await resolveUrl(e)
    playing.value = { ...e, _url: url }
  } catch (err) {
    accError.value = err.message || '获取播放地址失败'
  }
}

function closePlayer() {
  playing.value = null
}

onMounted(async () => {
  const saved = localStorage.getItem('ol-theme')
  isDark.value = saved
    ? saved === 'dark'
    : window.matchMedia?.('(prefers-color-scheme: dark)').matches
  applyTheme()

  await checkAuth()
  try {
    await loadAccounts()
  } catch {
    /* 401 时已弹出登录 */
  }
})
</script>

<style scoped>
.app-shell {
  min-height: 100vh;
}

.app-header {
  position: sticky;
  top: 0;
  z-index: 50;
  background: var(--ol-panel);
  border-bottom: 1px solid var(--ol-border);
}
.header-inner {
  max-width: 1180px;
  margin: 0 auto;
  padding: 0 20px;
  height: var(--ol-header-h);
  display: flex;
  align-items: center;
  gap: 28px;
}

.brand {
  display: flex;
  align-items: center;
  gap: 9px;
  cursor: pointer;
  user-select: none;
  flex-shrink: 0;
}
.brand-icon {
  width: 28px;
  height: 28px;
  border-radius: 8px;
  background: linear-gradient(135deg, #5b7dff, #7c4dff);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 6px;
  flex-shrink: 0;
}
.brand-name {
  font-size: 17px;
  font-weight: 700;
  letter-spacing: 0.2px;
  white-space: nowrap;
}

.app-nav {
  display: flex;
  gap: 4px;
  height: 100%;
  overflow-x: auto;
  scrollbar-width: none;
}
.app-nav::-webkit-scrollbar {
  display: none;
}
.nav-btn {
  height: 100%;
  padding: 0 4px;
  background: transparent;
  border: none;
  border-bottom: 2px solid transparent;
  color: var(--ol-text-dim);
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  margin: 0 12px;
  white-space: nowrap;
  flex-shrink: 0;
}
.nav-btn:hover {
  color: var(--ol-text);
}
.nav-btn.active {
  color: var(--ol-primary);
  border-bottom-color: var(--ol-primary);
}

.header-actions {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

@media (max-width: 560px) {
  .header-inner {
    gap: 14px;
    padding: 0 12px;
  }
  .brand-name {
    display: none;
  }
  .nav-btn {
    margin: 0 8px;
    font-size: 13px;
  }
}
</style>
