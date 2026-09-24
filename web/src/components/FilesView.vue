<template>
  <div class="page">
    <div v-if="accounts.length === 0" class="empty-state card">
      <Icon name="inbox" :size="40" />
      <p>还没有存储，先去 <a href="#" @click.prevent="$emit('go-accounts')">存储管理</a> 添加一个云盘账号</p>
    </div>

    <template v-else>
      <!-- 面包屑（对齐官方 Nav：🏠 首页 / xx / xx）+ 布局切换（对齐官方 header Layout） -->
      <div class="crumbs-row">
        <nav class="crumbs">
          <a href="#" class="crumb-link crumb-home" title="返回首页" @click.prevent="$emit('go-home')">
            🏠 首页
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
              <span v-else class="crumb-link crumb-current">{{ c.name }}</span>
            </template>
            <template v-if="preview">
              <span class="crumb-sep">/</span>
              <span class="crumb-link crumb-current">{{ preview.name }}</span>
            </template>
          </template>
        </nav>
        <div v-if="!preview" class="view-toggle" role="group" aria-label="视图切换">
          <button
            :class="{ active: viewMode === 'list' }"
            :title="viewMode === 'list' ? '切换为宫格' : '列表'"
            @click="$emit('update:view-mode', viewMode === 'list' ? 'grid' : 'list')"
          >
            <Icon :name="viewMode === 'list' ? 'grid' : 'list'" :size="16" />
          </button>
        </div>
      </div>

      <div v-if="err" class="alert alert-error">{{ err }}</div>

      <!-- PLACEHOLDER_REST -->
    </template>
  </div>
</template>
