# 2026-05-27 工作总结

## 今日目标

1. 恢复 Tauri updater 签名链路。
2. 让 GitHub Actions 同时产出：
   - Windows 安装包
   - macOS x86_64 安装包
   - updater 相关签名文件与 `latest.json` 平台信息
3. 安装并接通 `gh`，用于直接读取 Actions 原始日志，避免继续盲试。

## 今日最终结果

### 已确认成功

1. `gh` 已可在本机直接使用：
   - 路径：`/usr/local/bin/gh`
   - 版本：`2.92.0`
2. GitHub CLI 已完成登录，可直接读取 run / job / release / secret。
3. `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 已重新覆盖到仓库 secrets。
4. `test31` 成功通过 updater signing 自检：
   - Windows `Verify updater signing key`：成功
   - macOS `Verify updater signing key`：成功
5. `v3.14.1-test31` 已成功发布以下关键产物：
   - `CC-Switch-v3.14.1-test31-macOS-x86_64.dmg`
   - `CC-Switch-v3.14.1-test31-macOS-x86_64.zip`
   - `CC-Switch-v3.14.1-test31-macOS-x86_64.tar.gz`
   - `CC-Switch-v3.14.1-test31-macOS-x86_64.tar.gz.sig`
   - `CC-Switch-v3.14.1-test31-Windows-Setup.exe`
   - `CC-Switch-v3.14.1-test31-Windows-Setup.exe.sig`
   - `latest.json`

### 结论

这次真正打通的不只是“构建成功”，而是：

1. updater 私钥格式识别
2. updater 私钥密码匹配
3. GitHub Secrets 正确写入
4. workflow 内部自检逻辑
5. macOS x86_64 安装包产物发布

## 本次排查过程中的真实根因

### 1. 早期误判：以为是 Apple 证书 / notarization 问题

实际不是。

真正阻塞 macOS / Windows 共同失败的步骤是：

- `Verify updater signing key`

因此这类问题应先看 updater 签名，再看 Apple 签名、公证、DMG、`latest.json`。

### 2. `gh` 没装好，导致一直只能看摘要，拿不到原始报错

一开始只能看到：

- `Prepare Tauri signing key` 成功 / 失败
- `Verify updater signing key` 成功 / 失败

但看不到原始 stderr，排查效率很低。

最终通过以下方式解决：

1. 不再依赖 Homebrew 下载二进制包
2. 用 SSH 克隆 `cli/cli`
3. 用 Go 自动拉取新工具链
4. 本地源码编译 `gh`

### 3. GitHub Secrets 中的 updater key 格式反复出错

这次实际遇到过 4 种错误形态：

1. 把两行原文 secret 直接喂给 signer
   - 报错：`failed to decode base64 secret key: Invalid symbol 32`
   - 含义：空格被读进去了，signer 实际期待的是 base64 包裹内容
2. 从两行原文里只提取第二行
   - 报错：`invalid utf-8 sequence of 1 bytes from index 9`
   - 含义：不是 signer 期望的完整 key blob
3. base64 内容末尾多了换行
   - 报错：`Invalid symbol 10`
   - 含义：换行被当成非法字符
4. 私钥和密码不匹配
   - 报错：`incorrect updater private key password: Invalid input`

### 4. `gh secret set ... --body -` 的写法不稳

这次有一次通过 stdin 管道写 secret，结果 GitHub 上拿到的值不符合预期。

后面改成：

```bash
gh secret set NAME --repo zzktz/ts-switch --body "..."
```

才稳定。

## 最终确认的正确做法

### 一、仓库中实际可用的 updater key / password

本地验证成功的一对是：

#### `TAURI_SIGNING_PRIVATE_KEY`

```text
dW50cnVzdGVkIGNvbW1lbnQ6IHJzaWduIGVuY3J5cHRlZCBzZWNyZXQga2V5ClJXUlRZMEl5WTBaK3NDSE9pazluVVBpZjBTc1Y4UGxRWVZJSVFUVEtoZ3d0NVQ5eGlRWUFBQkFBQUFBQUFBQUFBQUlBQUFBQVMzdW5TR05rdVQ4TUVyOUs4OHB2NUttdExrQlQvV0J1VUVQQkFwMXBIdHBHTHp1M1RWeVJJdDhkSTBGRk1FOWlndkJBdjA5ajRQaUwxMDlFRnlZS0ZkU1JMMXFQdWhTSFAxbzBaZ1VmUjBaOTVnNEplOWh3M2M2NmRuTFB4ak4xNW1FUE9seTJSdG89Cg==
```

#### `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

```text
qY8sio+AlyERPG9AVK3V5yChht/3Njir
```

### 二、最关键的经验

这把 key 对 Tauri signer 来说，正确输入格式是：

- **单行 base64 包裹后的完整 key blob**

不是：

- 两行 `untrusted comment + 第二行`
- 也不是只取第二行 base64

### 三、workflow 最终采用的兼容策略

现在 `.github/workflows/release.yml` 已做兼容：

1. 如果 secret 是两行原文：
   - 先整体重新 `base64`
   - 再转成单行
2. 如果 secret 本身已经是单行 base64 包裹内容：
   - 直接使用
3. 自检时写入文件不追加多余换行

这样下次即使 GitHub Secret 文本框显示折行，也不容易再踩回同一个坑。

## 这次新增 / 关键提交

- `19eab8ef` `ci: pass tauri signing key content to builds`
- `d6869193` `ci: add updater signing selftest`
- `276e1ca5` `ci: normalize updater signing key format`
- `63c9a808` `ci: strip whitespace from updater key`
- `1193e6d0` `ci: avoid newline in updater selftest key`
- `49628963` `ci: use wrapped updater key format`

## 成功验证节点

### 关键测试标签

- `v3.14.1-test31`

### 关键 workflow run

- run id: `26494902114`

### 在 `test31` 中已确认成功的关键步骤

- `Prepare Tauri signing key`
- `Verify updater signing key`
- `Build Tauri App (Windows)`
- `Build Tauri App (macOS)`
- release 资产上传

## 本机 `gh` 安装经验

### 为什么 Homebrew 不适合继续死磕

这台机器上：

- `brew install gh`
- `brew install --build-from-source gh`

都卡在 GitHub 下载阶段。

### 可复用的解决方案

1. 确认本机有 `go`
2. 用 SSH 克隆：

```bash
git clone --depth 1 --branch v2.92.0 git@github.com:cli/cli.git /private/tmp/gh-src
```

3. 若 `go` 版本不够，使用：

```bash
GOSUMDB=sum.golang.org GOTOOLCHAIN=auto go version
```

4. 编译：

```bash
cd /private/tmp/gh-src
GOSUMDB=sum.golang.org GOTOOLCHAIN=auto make bin/gh
```

5. 安装：

```bash
install -m755 /private/tmp/gh-src/bin/gh /usr/local/bin/gh
```

## 下次遇到同类项目时的建议顺序

### 推荐顺序

1. 先装好并登录 `gh`
2. 先看 Actions 原始日志，不要先靠猜
3. 如果 macOS / Windows 同时在 very early step 失败：
   - 优先怀疑 updater signing
   - 不要先怀疑 notarization
4. 先做一个最小自检：
   - `pnpm tauri signer sign ...`
5. 先在本地验证 key / password 是否真的匹配
6. 再决定 GitHub Secret 应该存什么格式

### 不推荐顺序

1. 直接来回改 Apple 证书 / Team ID / notarization
2. 在拿不到原始日志时连续打 tag 盲试
3. 假设 Secret 输入框的“自动换行显示”就等于真实换行格式

## 当前状态

截至本日志写入时：

1. updater signing 问题已经打通
2. macOS x86_64 安装包已经确认发布成功
3. Windows 安装包与签名产物也已恢复
4. `latest.json` 已重新生成

## 建议补做

虽然核心链路已经恢复，后续仍建议补两件事：

1. 单独检查 `latest.json` 中 `platforms` 字段是否完整包含：
   - `darwin-x86_64`
   - `windows-x86_64`
2. 将本次可用的 updater secret 来源再做一份离线备份，避免后续再次误覆盖

