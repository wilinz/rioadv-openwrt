# rioadv

> 只发 **RIO**(Route Information Option, RFC 4191) 的 RA 小守护进程，为 OpenWrt 打造。

在 LAN 上周期发送路由通告，但**路由器寿命恒为 0、不带前缀**，只告诉链路上的主机"这几个 IPv6 网段交给我转发"。
不宣告默认路由，所以其余流量的走向完全不变。

## 为什么需要它

典型场景：上游只给路由器自己分 IPv6 地址、**不下发前缀(PD)**，LAN 只能靠 NAT66 出去；
而你只想让**个别网段**走 IPv6(比如某个只有 IPv6 入口可用的校内服务)，不想让全 LAN 的 IPv6 流量绕过透明代理直连。

OpenWrt 自带的 odhcpd 做不到：它的 RIO 只能从接口自身地址推导，没法加任意路由；
软件源里也没有 radvd，`uradvd` 只支持 /64 前缀、不支持 RIO。

## 组成

| 包 | 内容 |
|----|------|
| `rioadv` | Rust 引擎 `/usr/sbin/rioadv`(静态 musl) + procd 服务 + UCI 配置 |
| `luci-app-rioadv` | LuCI 界面(网络 → RIO 路由通告) |

```
rioadv-rs/             Rust 源码(只依赖 socket2)
pkg/rioadv/            引擎包(CONTROL + data 安装树)
pkg/luci-app-rioadv/   LuCI 包
build.sh               交叉编译 + 打包两个 .ipk(无需 OpenWrt SDK)
```

## 构建

依赖 [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) + zig：

```sh
./build.sh                                              # x86_64
TARGET=aarch64-unknown-linux-musl ARCH=aarch64_generic ./build.sh
TARGET=mipsel-unknown-linux-musl ARCH=mipsel_24kc BUILDSTD=1 ./build.sh   # tier-3, 需 nightly
```

产物在 `out/`。

## 安装与配置

```sh
opkg install rioadv_*_<arch>.ipk luci-app-rioadv_*_all.ipk
```

装完默认**不启用**。在 LuCI 里填好网段后打开，或直接改 `/etc/config/rioadv`：

```
config rioadv 'main'
	option enabled '1'
	option interface 'br-lan'
	option dhcp_section 'lan'      # M/O 标志跟随 dhcp.lan.ra_flags, 与 odhcpd 一致
	option lifetime '1800'         # 路由寿命, 应 ≥ 3 × interval
	option interval '60'
	list route '2001:250:3402::/48'
```

```sh
/etc/init.d/rioadv restart
```

主机要能真正走通这些网段，路由器得能转发出去，例如 NAT66：

```sh
uci set firewall.@zone[1].masq6='1'        # wan 区, 按实际 zone 调整
uci set network.wan6.sourcefilter='0'      # 否则 WAN 的 v6 默认路由只放行路由器自己的源地址
uci commit && /etc/init.d/firewall reload && ifup wan6
```

## 行为说明

- 启动后先以 4 秒间隔连发 3 次，之后按 `interval` 周期发送。
- 服务停止时补发一次 lifetime=0 的 RA，主机立即撤销路由，不用等过期。
- 它发的 RA 路由器寿命恒为 0：若以后让 odhcpd 宣告默认路由，本服务会把它抵消，届时应停用本服务。
- 主机端支持情况：Linux(NetworkManager / systemd-networkd)、Windows、Android 会接收 RIO；
  Linux 内核自己处理 RA 时需 `accept_ra_rt_info_max_plen` ≥ 前缀长度(默认 0 = 忽略)。

## License

MIT
