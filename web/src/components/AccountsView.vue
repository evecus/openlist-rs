<template>
  <div class="page narrow">
    <section class="card panel">
      <div class="panel-head">
        <Icon :name="editingId ? 'edit' : 'folder-plus'" :size="18" />
        <h2>{{ editingId ? '编辑存储' : '添加存储' }}</h2>
        <span v-if="editingId && loadingSecret" class="secret-status loading">
          <span class="spin loader-sm"></span>
          正在加载凭据…
        </span>
        <span v-else-if="editingId && secretError" class="secret-status error">
          {{ secretError }}
        </span>
        <button v-if="editingId" class="cancel-edit-btn" @click="cancelEdit" title="取消编辑">
          <Icon name="x" :size="15" />
          取消
        </button>
      </div>
      <div class="panel-body">
        <div class="field">
          <label>网盘类型</label>
          <select class="input" v-model="form.driver" :disabled="!!editingId">
            <option v-for="d in DRIVERS" :key="d.value" :value="d.value">{{ d.label }}</option>
          </select>
        </div>
        <div class="field">
          <label>备注名（可选）</label>
          <input class="input" v-model="form.name" :placeholder="`如：我的${currentDriverLabel}`" />
        </div>

        <template v-if="form.driver === 'quark' || form.driver === 'quark_uc'">
          <div class="field">
            <label>Cookie</label>
            <textarea
              class="input"
              v-model="form.cookie"
              :placeholder="form.driver === 'quark'
                ? '登录 pan.quark.cn 后，F12 -> 网络 -> 任意请求 -> 复制请求头里的 Cookie 整段粘贴到这里'
                : '登录 drive.uc.cn 后，F12 -> 网络 -> 任意请求 -> 复制请求头里的 Cookie 整段粘贴到这里'"
            ></textarea>
          </div>
          <p class="hint-text">
            获取方式：浏览器登录{{ form.driver === 'quark' ? '夸克网盘' : 'UC 网盘' }} → F12 开发者工具 → Network → 刷新 → 任选一个网盘请求 → 复制 Request Headers 中完整的 Cookie 值。
          </p>
        </template>

        <template v-else-if="form.driver === '123pan'">
          <div class="field">
            <label>账号（手机号或邮箱）</label>
            <input class="input" v-model="form.username" placeholder="123 网盘登录账号" />
          </div>
          <div class="field">
            <label>密码</label>
            <input class="input" type="password" v-model="form.password" placeholder="登录密码" />
          </div>
        </template>

        <template v-else-if="form.driver === 'aliyundrive_open'">
          <div class="field">
            <label>RefreshToken</label>
            <textarea class="input" v-model="form.refresh_token" placeholder="阿里云盘开放平台 refresh_token"></textarea>
          </div>
          <p class="hint-text">
            获取方式：登录 aliyun.com 后访问 alipan.com，F12 → 应用 → 本地存储 → token 里的 refresh_token；或使用第三方令牌获取工具。
          </p>
        </template>

        <template v-else-if="form.driver === 'baidu_netdisk'">
          <div class="field">
            <label>RefreshToken</label>
            <textarea class="input" v-model="form.refresh_token" placeholder="百度网盘开放平台 refresh_token"></textarea>
          </div>
          <p class="hint-text">获取方式：使用 OpenList 官方扫码授权工具获取百度网盘 refresh_token 后粘贴到这里。</p>
        </template>

        <template v-else-if="form.driver === 'pan115'">
          <div class="field">
            <label>Cookie</label>
            <textarea class="input" v-model="form.cookie" placeholder="需包含 UID、CID、SEID 三个字段"></textarea>
          </div>
          <p class="hint-text">获取方式：浏览器登录 115.com → F12 → 网络 → 刷新 → 复制 Request Headers 中的完整 Cookie（须含 UID/CID/SEID）。</p>
        </template>

        <template v-else-if="form.driver === 'thunder'">
          <div class="field">
            <label>账号（手机号或邮箱）</label>
            <input class="input" v-model="form.username" placeholder="迅雷登录账号" />
          </div>
          <div class="field">
            <label>密码</label>
            <input class="input" type="password" v-model="form.password" placeholder="登录密码" />
          </div>
          <p class="hint-text">首次登录可能需要短信验证，验证链接会返回在错误信息中。</p>
        </template>

        <template v-else-if="form.driver === 'lanzou'">
          <div class="field">
            <label>Cookie</label>
            <textarea class="input" v-model="form.cookie" placeholder="登录 pc.woozooo.com 后的 cookie"></textarea>
          </div>
          <p class="hint-text">获取方式：浏览器登录蓝奏云 pc.woozooo.com → F12 → 网络 → 刷新 → 复制请求头中的 Cookie（约 15 天有效）。</p>
        </template>

        <!-- 服务器代理开关 -->
        <div class="field proxy-field">
          <label class="proxy-label">
            <span class="proxy-label-text">
              <span class="proxy-title">服务器代理</span>
              <span class="proxy-desc">开启后下载流量经本服务中转；关闭则直接跳转网盘直链（默认关闭）</span>
            </span>
            <span
              class="toggle"
              :class="{ on: form.server_proxy }"
              @click="form.server_proxy = !form.server_proxy"
              role="switch"
              :aria-checked="form.server_proxy"
            >
              <span class="toggle-thumb"></span>
            </span>
          </label>
        </div>

        <div v-if="error" class="alert alert-error">{{ error }}</div>
        <button class="btn" :class="{ 'btn-edit': !!editingId }" :disabled="adding" @click="submit">
          <span v-if="adding" class="spin loader"></span>
          <template v-if="!adding">
            {{ editingId ? '保存修改' : '添加存储' }}
          </template>
          <template v-else>
            {{ editingId ? '保存中…' : '验证中…' }}
          </template>
        </button>
      </div>
    </section>

    <section class="card panel">
      <div class="panel-head">
        <Icon name="cloud" :size="18" />
        <h2>已添加存储</h2>
      </div>
      <div class="panel-body no-pad">
        <div v-if="accounts.length === 0" class="empty-mini">
          <Icon name="inbox" :size="28" />
          <span>暂无存储，请在上方添加</span>
        </div>
        <div
          v-for="a in accounts"
          :key="a.id"
          class="account-row"
          :class="{ editing: editingId === a.id }"
        >
          <span class="badge" :style="badgeStyle(a.driver)">{{ badgeName(a.driver) }}</span>
          <span class="name">{{ a.name }}</span>
          <span v-if="a.server_proxy" class="proxy-badge" title="服务器代理已开启">代理</span>
          <button
            class="btn-icon btn-edit-icon"
            :class="{ active: editingId === a.id }"
            title="编辑"
            @click="startEdit(a)"
          >
            <Icon name="edit" :size="15" />
          </button>
          <button class="btn-icon btn-danger" title="删除" @click="$emit('delete', a)">
            <Icon name="trash" :size="16" />
          </button>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup>
import { computed, reactive, ref, watch, nextTick } from 'vue'
import Icon from './Icon.vue'

const props = defineProps({
  accounts: { type: Array, required: true },
  error: { type: String, default: '' },
  adding: { type: Boolean, default: false },
  driverLabels: { type: Object, required: true },
  fetchSecret: { type: Function, default: null }
})
const emit = defineEmits(['add', 'edit', 'delete'])

const DRIVERS = [
  { value: 'quark', label: '夸克网盘' },
  { value: 'quark_uc', label: 'UC 网盘' },
  { value: '123pan', label: '123 网盘' },
  { value: 'aliyundrive_open', label: '阿里云盘' },
  { value: 'baidu_netdisk', label: '百度网盘' },
  { value: 'pan115', label: '115 网盘' },
  { value: 'thunder', label: '迅雷网盘' },
  { value: 'lanzou', label: '蓝奏云' }
]

// 当前正在编辑的账号 id，null 表示新增模式
const editingId = ref(null)

const form = reactive({
  driver: 'quark',
  name: '',
  cookie: '',
  username: '',
  password: '',
  refresh_token: '',
  server_proxy: false
})

const currentDriverLabel = computed(
  () => DRIVERS.find((d) => d.value === form.driver)?.label.replace('网盘', '') || ''
)

function resetForm() {
  editingId.value = null
  form.driver = 'quark'
  form.name = ''
  form.cookie = ''
  form.username = ''
  form.password = ''
  form.refresh_token = ''
  form.server_proxy = false
}

// 编辑回填中的 loading 状态（用于按钮禁用/提示）
const loadingSecret = ref(false)
const secretError = ref('')

// 点击编辑按钮：回填信息到表单（包括凭据，如 cookie/密码/refresh_token）
async function startEdit(account) {
  // 如果点的是已在编辑的同一个，取消
  if (editingId.value === account.id) {
    cancelEdit()
    return
  }
  editingId.value = account.id
  secretError.value = ''
  // 先用列表里已有的字段回填，不用等接口
  form.driver = account.driver
  form.name = account.name
  form.cookie = ''
  form.username = ''
  form.password = ''
  form.refresh_token = ''
  form.server_proxy = account.server_proxy ?? false
  // 滚动到顶部方便填写
  nextTick(() => {
    window.scrollTo({ top: 0, behavior: 'smooth' })
  })

  // 再拉取凭据明细，回填 cookie / 账号密码 / refresh_token
  if (!props.fetchSecret) return
  loadingSecret.value = true
  try {
    const secret = await props.fetchSecret(account.id)
    // 若在请求过程中用户切换了编辑目标或取消了编辑，则丢弃这次结果
    if (editingId.value !== account.id) return
    form.driver = secret.driver ?? form.driver
    form.name = secret.name ?? form.name
    form.cookie = secret.cookie ?? ''
    form.username = secret.username ?? ''
    form.password = secret.password ?? ''
    form.refresh_token = secret.refresh_token ?? ''
    form.server_proxy = secret.server_proxy ?? form.server_proxy
  } catch (e) {
    secretError.value = e.message || '获取凭据失败，请手动重新填写'
  } finally {
    loadingSecret.value = false
  }
}

function cancelEdit() {
  resetForm()
  secretError.value = ''
}

function submit() {
  if (editingId.value) {
    emit('edit', { id: editingId.value, ...form })
  } else {
    emit('add', form)
  }
}

// 操作成功后重置（账号列表变化 = 成功信号）
watch(
  () => props.accounts.length,
  () => {
    resetForm()
  }
)

// 编辑成功：error 清空且 adding 从 true→false 时也重置
// （通过 App.vue 在成功后清空 editingId 等即可，这里监听 adding 结束 + 无 error）
watch(
  () => props.adding,
  (val) => {
    if (!val && !props.error && editingId.value) {
      resetForm()
    }
  }
)

const DRIVER_COLORS = {
  quark: '#00b578',
  quark_uc: '#0089ff',
  '123pan': '#ff7d00',
  aliyundrive_open: '#722ed1',
  baidu_netdisk: '#2468f2',
  pan115: '#f53f3f',
  thunder: '#00b4d8',
  lanzou: '#86909c'
}
const badgeName = (d) => props.driverLabels[d] || d
const badgeStyle = (d) => {
  const c = DRIVER_COLORS[d] || '#86909c'
  return { background: c + '1f', color: c }
}
</script>

<style scoped>
.page.narrow {
  max-width: 620px;
  margin: 0 auto;
  padding: 28px 20px 60px;
}
.panel {
  margin-bottom: 20px;
  overflow: hidden;
}
.panel-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 16px 20px;
  border-bottom: 1px solid var(--ol-border);
  color: var(--ol-primary);
}
.panel-head h2 {
  font-size: 15px;
  font-weight: 600;
  color: var(--ol-text);
  margin: 0;
  flex: 1;
}

/* 取消编辑按钮 */
.cancel-edit-btn {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 3px 10px;
  border: 1px solid var(--ol-border);
  border-radius: 6px;
  background: transparent;
  color: var(--ol-text-dim);
  font-size: 12px;
  cursor: pointer;
  transition: color 0.15s, border-color 0.15s;
}
.cancel-edit-btn:hover {
  color: var(--ol-text);
  border-color: var(--ol-text-dim);
}

.panel-body {
  padding: 20px;
}
.panel-body.no-pad {
  padding: 6px 20px;
}

.loader {
  width: 13px;
  height: 13px;
  border: 2px solid rgba(255, 255, 255, 0.4);
  border-top-color: #fff;
  border-radius: 50%;
}

/* 凭据加载状态提示 */
.secret-status {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  white-space: nowrap;
}
.secret-status.loading {
  color: var(--ol-text-dim);
}
.secret-status.error {
  color: var(--ol-danger);
}
.loader-sm {
  width: 11px;
  height: 11px;
  border: 2px solid color-mix(in srgb, var(--ol-text-dim) 30%, transparent);
  border-top-color: var(--ol-text-dim);
  border-radius: 50%;
  flex-shrink: 0;
}

/* 保存修改按钮用橙色区分 */
.btn.btn-edit {
  background: #f59e0b;
}
.btn.btn-edit:hover:not(:disabled) {
  background: #d97706;
}

.empty-mini {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  color: var(--ol-text-dim);
  padding: 30px 0;
  font-size: 13px;
}

.account-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 0;
  border-bottom: 1px solid var(--ol-border);
  transition: background 0.15s;
  border-radius: 4px;
}
.account-row:last-child {
  border-bottom: none;
}
.account-row.editing {
  background: color-mix(in srgb, var(--ol-primary) 6%, transparent);
  padding-left: 8px;
  padding-right: 8px;
  margin: 0 -8px;
}
.account-row .name {
  flex: 1;
  font-size: 13.5px;
}

/* 编辑按钮：样式与删除按钮保持一致，无背景/边框 */
.btn-edit-icon {
  background: transparent;
  border: 1px solid transparent;
  color: var(--ol-text-dim);
  cursor: pointer;
  transition: background-color 0.15s, color 0.15s;
}
.btn-edit-icon:hover,
.btn-edit-icon.active {
  background: color-mix(in srgb, var(--ol-primary) 10%, transparent);
  color: var(--ol-primary);
}

/* 服务器代理开关 */
.proxy-field {
  margin-top: 4px;
}
.proxy-label {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  cursor: default;
}
.proxy-label-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.proxy-title {
  font-size: 13.5px;
  font-weight: 500;
  color: var(--ol-text);
}
.proxy-desc {
  font-size: 12px;
  color: var(--ol-text-dim);
  line-height: 1.4;
}

/* Toggle 开关 */
.toggle {
  flex-shrink: 0;
  width: 42px;
  height: 24px;
  border-radius: 12px;
  background: var(--ol-border);
  position: relative;
  cursor: pointer;
  transition: background 0.2s;
  user-select: none;
}
.toggle.on {
  background: var(--ol-primary);
}
.toggle-thumb {
  position: absolute;
  top: 3px;
  left: 3px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: #fff;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
  transition: transform 0.2s;
}
.toggle.on .toggle-thumb {
  transform: translateX(18px);
}

/* 代理状态角标 */
.proxy-badge {
  font-size: 11px;
  padding: 1px 6px;
  border-radius: 4px;
  background: #e6f4ff;
  color: #1677ff;
  font-weight: 500;
  flex-shrink: 0;
}
</style>
