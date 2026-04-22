# 发布流程

## 核心原则

**一个版本号 = 一次构建 = 一个 SHA256。**

绝对不能 force-push 已发布的 tag。tag 一旦推出去，对应的 binary 就固定了；如果重打 tag，Homebrew formula 里的 SHA256 会和新 binary 对不上，导致 `brew install/upgrade` 失败。

---

## 标准发布流程

```
dev 分支开发
    ↓
1. bump Cargo.toml 版本号
2. 合并到 main
3. 打 tag → push
    ↓
GitHub Actions: 4 平台编译 → 创建 Release → 更新 Homebrew formula
    ↓
brew upgrade
```

### Step 1：确认版本号

在 `dev` 分支上，**先升版本号再合并**：

```bash
# 编辑 Cargo.toml，把 version = "x.y.z" 改成新版本
# 然后提交
git add Cargo.toml
git commit -m "chore: bump workspace version to x.y.z"
```

版本号规则：只能往前走，不能复用。`sage --version` 输出来自 `Cargo.toml`，和 tag 必须一致。

### Step 2：合并到 main

```bash
git checkout main
git merge dev
git push origin main
git checkout dev  # 回到 dev 继续开发
```

### Step 3：打 tag 并推送（触发 release）

```bash
git tag vx.y.z
git push origin vx.y.z
```

**等 Release workflow 全部通过**（约 5–10 分钟）。关键 job 顺序：
1. `Build` — 4 平台并行编译，生成 `.tar.gz` + `.sha256`
2. `Publish release` — 上传到 GitHub Release，用新 SHA256 更新 Homebrew tap

### Step 4：本地更新

```bash
# 先确认 Actions 全绿（https://github.com/chasey-myagi/sage/actions）
brew upgrade chasey-myagi/tap/sage
sage --version  # 应该输出新版本号
```

---

## 已知坑 & 应对

### 坑 1：忘了 bump Cargo.toml

**现象**：`brew upgrade` 后 `sage --version` 还是老版本号。

**原因**：binary 里的版本字符串来自编译时的 `Cargo.toml`，tag 名只是标识符。

**修法**：不能在已发布 tag 上修，必须发下一个版本（bump + 新 tag）。

---

### 坑 2：CI 失败后 force-push 了 tag

**现象**：`brew reinstall` 报 SHA256 不匹配。

**原因**：同一 tag 名指向了不同的 commit → 不同的 binary → 不同的 SHA256，但 Homebrew tap 的 formula 只更新了最后一次（或某一次）。

**修法**：手动更新本地 formula，然后发下一个真正的版本（不要再 force-push）。

手动修 formula 的方法（仅限紧急救场）：
```bash
# 计算当前 release 里每个平台的实际 SHA256
curl -sL https://github.com/chasey-myagi/sage/releases/download/vX.Y.Z/sage-vX.Y.Z-darwin-arm64.tar.gz \
  | shasum -a 256

# 直接编辑本地 tap 的 formula
vi /opt/homebrew/Library/Taps/chasey-myagi/homebrew-tap/Formula/sage.rb

brew reinstall chasey-myagi/tap/sage
```

**根本教训：** tag 一旦推出就不能动。CI 有问题只修 workflow，bump patch 版本重新发，不 force-push。

---

### 坑 3：release workflow 的 perl sha256 替换 bug

**已修复**（见 `.github/workflows/release.yml`）。

原始 bug：`perl -i -0pe "s|...|\\1${SHA}\\2|g"` — shell 展开后 `\1` 紧接 hash 开头的数字，被 perl 解析为八进制转义（如 `\177` = DEL）。

修复方案：SHA 通过环境变量传入，perl 内用 `$ENV{SHA}` 读取，避免替换字符串里出现 `\1XX` 形式。

---

### 坑 4：本地 rustfmt 版本与 CI 不一致

**已修复**（`rust-toolchain.toml` 锁定版本）。

CI 用 `RUSTFLAGS=-Dwarnings`，所有 warning 都是 error。本地不同版本的 rustfmt 格式化结果可能不同，导致 CI 格式检查失败。

**规则**：`rust-toolchain.toml` 锁定的版本就是本地开发版本，提交前运行 `cargo fmt --all -- --check` 确认。

---

### 坑 5：cross-compile target 未安装

**已修复**（`release.yml` 增加 `rustup target add` 步骤）。

`rust-toolchain.toml` 声明的 channel 不带 targets，`dtolnay/rust-toolchain` 给 `stable` 安装的 target 对 pinned channel 无效。

**规则**：workflow 里必须有显式的 `rustup target add ${{ matrix.target }}` 步骤。

---

## 快速检查清单

发版前确认：

- [ ] `Cargo.toml` 版本号已 bump，与 tag 名一致
- [ ] `main` 分支已包含所有要发的内容
- [ ] 本地 `cargo fmt --all -- --check` 无报错
- [ ] 本地 `RUSTFLAGS=-Dwarnings cargo clippy --workspace --all-targets` 无报错
- [ ] 本地 `cargo test --workspace` 全绿

发版后确认：

- [ ] GitHub Actions Release workflow 全绿
- [ ] `brew upgrade` 后 `sage --version` 输出正确版本号

---

## 本地 dev build（不走 Homebrew）

```bash
cargo run -p coding-agent                              # TUI 交互模式
echo "hello" | cargo run -p coding-agent -- --print   # print 模式
```
