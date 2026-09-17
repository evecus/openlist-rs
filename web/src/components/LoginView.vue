<template>
  <div class="login-screen">
    <div class="login-bg">
      <div class="blob blob-1"></div>
      <div class="blob blob-2"></div>
    </div>

    <div class="login-card">
      <div class="brand">
        <div class="brand-mark">
          <svg viewBox="0 0 48 48" fill="none">
            <path
              d="M6 12a2 2 0 0 1 2-2h10.5l3 4H40a2 2 0 0 1 2 2v20a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V12Z"
              fill="#fff"
              fill-opacity="0.92"
            />
            <path d="M6 16h36v16a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V16Z" fill="#fff" />
          </svg>
        </div>
        <h1>OpenList</h1>
        <p>登录以管理你的云盘文件</p>
      </div>

      <form @submit.prevent="$emit('login')">
        <div class="field">
          <label>用户名</label>
          <input
            class="input"
            :value="form.username"
            @input="$emit('update:form', { ...form, username: $event.target.value })"
            autocomplete="username"
            placeholder="请输入用户名"
          />
        </div>
        <div class="field">
          <label>密码</label>
          <input
            class="input"
            type="password"
            :value="form.password"
            @input="$emit('update:form', { ...form, password: $event.target.value })"
            autocomplete="current-password"
            placeholder="请输入密码"
          />
        </div>

        <div v-if="error" class="alert alert-error">{{ error }}</div>

        <button type="submit" class="btn btn-block login-btn" :disabled="loading">
          <span v-if="loading" class="spin loader"></span>
          {{ loading ? "登录中…" : "登 录" }}
        </button>
      </form>
    </div>

    <div class="login-footer">Powered by OpenList · Rust Edition</div>
  </div>
</template>

<script setup>
defineProps({
  form: { type: Object, required: true },
  error: { type: String, default: '' },
  loading: { type: Boolean, default: false }
})
defineEmits(['login', 'update:form'])
</script>

<style scoped>
.login-screen {
  position: fixed;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  background: var(--ol-bg);
  overflow: hidden;
  z-index: 200;
}

.login-bg {
  position: absolute;
  inset: 0;
  z-index: 0;
}
.blob {
  position: absolute;
  border-radius: 50%;
  filter: blur(80px);
  opacity: 0.35;
}
.blob-1 {
  width: 480px;
  height: 480px;
  background: #4b69fd;
  top: -160px;
  left: -120px;
}
.blob-2 {
  width: 420px;
  height: 420px;
  background: #7c4dff;
  bottom: -160px;
  right: -100px;
}
html.dark .blob {
  opacity: 0.2;
}

.login-card {
  position: relative;
  z-index: 1;
  width: 380px;
  max-width: calc(100vw - 40px);
  background: var(--ol-panel);
  border: 1px solid var(--ol-border);
  border-radius: 16px;
  box-shadow: var(--ol-shadow-lg);
  padding: 40px 36px 32px;
}

.brand {
  text-align: center;
  margin-bottom: 28px;
}
.brand-mark {
  width: 52px;
  height: 52px;
  margin: 0 auto 14px;
  border-radius: 14px;
  background: linear-gradient(135deg, #5b7dff, #7c4dff);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 11px;
  box-shadow: 0 8px 20px rgba(75, 105, 253, 0.35);
}
.brand h1 {
  font-size: 22px;
  font-weight: 700;
  margin: 0 0 4px;
  letter-spacing: 0.2px;
}
.brand p {
  margin: 0;
  font-size: 13px;
  color: var(--ol-text-dim);
}

.login-btn {
  margin-top: 6px;
  height: 42px;
  font-size: 15px;
}
.loader {
  width: 14px;
  height: 14px;
  border: 2px solid rgba(255, 255, 255, 0.4);
  border-top-color: #fff;
  border-radius: 50%;
}

.login-footer {
  position: relative;
  z-index: 1;
  margin-top: 24px;
  font-size: 12px;
  color: var(--ol-text-faint);
}
</style>
