# SSH Host Manager

这是一个本地桌面图形化工具，用来管理：

- 哪台服务器对应哪个 SSH key
- 每个 Host 的别名、地址、用户、端口
- 自动生成 OpenSSH 配置片段
- 直接把程序管理的 Host 区块写进你的主 SSH 配置文件

## 现在能做什么

- 左侧查看服务器列表
- 右侧编辑服务器信息
- 保存到本地 TOML 存储文件
- 实时预览生成的 SSH config
- 直接写入主 SSH 配置文件，并在每次写入前自动备份
- 从现有 SSH 配置导入简单 Host 配置
- 过滤主机列表
- 检查 key 文件是否存在
- 复制已有 Host 作为新条目

## 支持平台

- macOS
- Linux
- Windows

默认主 SSH 配置路径：

- macOS / Linux: `~/.ssh/config`
- Windows: `%USERPROFILE%\.ssh\config`

程序内部已经按平台处理默认路径和界面提示。

## 运行

```bash
cargo run
```

## 使用方式

1. 打开程序
2. 点击 `Add Host`
3. 填写：
   - Alias: SSH 别名
   - HostName / IP: 服务器地址
   - User: 登录用户
   - Port: 端口，可空
   - IdentityFile: 私钥路径，可以点 `Choose File`
   - Note: 备注
4. 点击 `Create Host`
5. 点击 `Apply Managed Hosts Now`

之后就可以直接：

```bash
ssh 你填写的Alias
```

## 本地数据文件

默认保存到：

- macOS / Linux: `~/.config/rurs_test/hosts.toml`
- Windows: 由系统配置目录决定，一般在用户配置目录下

## 主配置写入方式

程序不会粗暴覆盖整个 SSH 配置，而是：

- 读取现有主配置文件
- 在其中维护一段由程序托管的 Host 区块
- 每次写入前先创建 `config.bak-时间戳`
- 保留你手写的其他 SSH 配置内容

## Git 提交前建议

先本地跑：

```bash
cargo fmt
cargo test
cargo clippy --all-targets --all-features
```

仓库里已经附带 GitHub Actions，会在 Linux、Windows、macOS 上自动跑这些检查。
