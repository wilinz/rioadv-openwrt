// rioadv: 在指定网卡上周期发送只带 RIO(Route Information, RFC 4191) 的 RA。
//
// 路由器寿命恒为 0, 不宣告默认路由, 也不带前缀; 只告诉链路上的主机
// "这几个网段走我"。用于上游不给前缀下发、只能 NAT66 的场景下,
// 把个别 IPv6 网段(比如学校的 /48)引到网关, 而不改变其余流量的走向。
// odhcpd 的 RIO 只能从接口自身地址推导, 没法加任意路由, 所以单独发。

use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::net::{Ipv6Addr, SocketAddrV6};
use std::{env, fs, process, thread, time::Duration};

const ALL_NODES: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1);
// RFC 4861: 启动后先快速发几次, 让已在线的主机尽快学到路由
const INITIAL_ADVERTS: u32 = 3;
const INITIAL_INTERVAL: u64 = 4;

struct Route {
    prefix: Ipv6Addr,
    plen: u8,
}

struct Opts {
    iface: String,
    routes: Vec<Route>,
    lifetime: u32,
    interval: u64,
    managed: bool,
    other: bool,
    once: bool,
}

fn usage() -> ! {
    eprintln!(
        "usage: rioadv -i <iface> -r <prefix/len> [-r ...] [--lifetime <s>] [--interval <s>]\n\
         \x20             [--managed] [--other] [--once]\n\
         \x20 --lifetime  RIO 路由寿命, 默认 1800; 0 = 撤销路由\n\
         \x20 --interval  发送间隔, 默认 60\n\
         \x20 --managed/--other  RA 的 M/O 标志, 应与同链路的 odhcpd 保持一致\n\
         \x20 --once      只发一次就退出(配合 --lifetime 0 用于撤销)"
    );
    process::exit(2)
}

fn parse_route(s: &str) -> Route {
    let (addr, plen) = s.split_once('/').unwrap_or_else(|| {
        eprintln!("rioadv: 路由缺少前缀长度: {s}");
        process::exit(2)
    });
    let addr: Ipv6Addr = addr.parse().unwrap_or_else(|_| {
        eprintln!("rioadv: 无效的 IPv6 地址: {addr}");
        process::exit(2)
    });
    let plen: u8 = match plen.parse() {
        Ok(n) if n <= 128 => n,
        _ => {
            eprintln!("rioadv: 无效的前缀长度: {plen}");
            process::exit(2)
        }
    };
    // 清掉主机位, RFC 4191 要求前缀之后的位为 0
    let mask = if plen == 0 { 0 } else { u128::MAX << (128 - plen) };
    Route {
        prefix: Ipv6Addr::from(u128::from(addr) & mask),
        plen,
    }
}

fn parse_args() -> Opts {
    let mut o = Opts {
        iface: String::new(),
        routes: Vec::new(),
        lifetime: 1800,
        interval: 60,
        managed: false,
        other: false,
        once: false,
    };
    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "-i" => o.iface = val(),
            "-r" => o.routes.push(parse_route(&val())),
            "--lifetime" => o.lifetime = val().parse().unwrap_or_else(|_| usage()),
            "--interval" => o.interval = val().parse().unwrap_or_else(|_| usage()),
            "--managed" => o.managed = true,
            "--other" => o.other = true,
            "--once" => o.once = true,
            _ => usage(),
        }
    }
    if o.iface.is_empty() || o.routes.is_empty() || o.interval == 0 {
        usage()
    }
    o
}

fn build_ra(o: &Opts) -> Vec<u8> {
    let flags = if o.managed { 0x80 } else { 0 } | if o.other { 0x40 } else { 0 };
    // type=134 code=0 checksum(内核填) hop_limit=64 flags router_lifetime=0
    // reachable_time=0 retrans_timer=0 (0 = 不指定, 不覆盖 odhcpd 的值)
    let mut b = vec![134, 0, 0, 0, 64, flags, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    for r in &o.routes {
        // RIO 的前缀字段按需截断: /0 不带, /1-64 带 8 字节, /65-128 带 16 字节
        let pbytes = (r.plen as usize).div_ceil(64) * 8;
        b.push(24);
        b.push((1 + pbytes / 8) as u8);
        b.push(r.plen);
        b.push(0); // Prf = medium
        b.extend_from_slice(&o.lifetime.to_be_bytes());
        b.extend_from_slice(&r.prefix.octets()[..pbytes]);
    }
    b
}

fn ifindex(iface: &str) -> u32 {
    fs::read_to_string(format!("/sys/class/net/{iface}/ifindex"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_else(|| {
            eprintln!("rioadv: 网卡不存在: {iface}");
            process::exit(1)
        })
}

fn open_socket(iface: &str, idx: u32) -> Socket {
    let s = Socket::new(Domain::IPV6, Type::RAW, Some(Protocol::ICMPV6)).unwrap_or_else(|e| {
        eprintln!("rioadv: 创建 ICMPv6 套接字失败(需要 root): {e}");
        process::exit(1)
    });
    // RFC 4861: RA 的 hop limit 必须是 255, 否则主机会丢弃
    let setup = s
        .set_multicast_hops_v6(255)
        .and_then(|_| s.set_unicast_hops_v6(255))
        .and_then(|_| s.set_multicast_if_v6(idx))
        .and_then(|_| s.set_multicast_loop_v6(false))
        .and_then(|_| s.bind_device(Some(iface.as_bytes())));
    if let Err(e) = setup {
        eprintln!("rioadv: 设置套接字失败: {e}");
        process::exit(1)
    }
    s
}

fn main() {
    let o = parse_args();
    let idx = ifindex(&o.iface);
    let sock = open_socket(&o.iface, idx);
    let pkt = build_ra(&o);
    let dst = SockAddr::from(SocketAddrV6::new(ALL_NODES, 0, 0, idx));

    let desc: Vec<String> = o.routes.iter().map(|r| format!("{}/{}", r.prefix, r.plen)).collect();
    eprintln!(
        "rioadv: {} 上宣告 {} (lifetime={}s, interval={}s, M={} O={})",
        o.iface,
        desc.join(" "),
        o.lifetime,
        o.interval,
        o.managed as u8,
        o.other as u8
    );

    let mut sent = 0u32;
    loop {
        // 网卡临时 down 时发送会失败, 记日志后继续, 等它回来
        if let Err(e) = sock.send_to(&pkt, &dst) {
            eprintln!("rioadv: 发送失败: {e}");
        }
        sent += 1;
        if o.once {
            return;
        }
        let wait = if sent < INITIAL_ADVERTS { INITIAL_INTERVAL } else { o.interval };
        thread::sleep(Duration::from_secs(wait));
    }
}
