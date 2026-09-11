'use strict';
'require view';
'require form';
'require uci';
'require rpc';
'require poll';
'require dom';
'require tools.widgets as widgets';

var callServiceList = rpc.declare({
	object: 'service',
	method: 'list',
	params: [ 'name' ],
	expect: { '': {} }
});

// 保存后 form.Map 会重新渲染各 section, 轮询只注册一次
var polling = false;

function instanceOf(res) {
	var inst = res && res.rioadv && res.rioadv.instances;
	for (var k in inst)
		return inst[k];
	return null;
}

function renderStatus(res) {
	var i = instanceOf(res);
	if (!i || !i.running)
		return E('span', { 'style': 'color:#dc2626; font-weight:600' },
			uci.get('rioadv', 'main', 'enabled') == '1' ? _('未运行') : _('已停用'));
	return E('div', {}, [
		E('span', { 'style': 'color:#16a34a; font-weight:600' }, _('运行中') + ' (PID ' + i.pid + ')'),
		E('pre', { 'style': 'margin:6px 0 0; white-space:pre-wrap; font-size:12px; opacity:.72' },
			(i.command || []).join(' '))
	]);
}

return view.extend({
	load: function () {
		return Promise.all([
			callServiceList('rioadv'),
			uci.load('dhcp')
		]);
	},

	render: function (data) {
		var m, s, o;

		m = new form.Map('rioadv', _('RIO 路由通告'),
			_('在 LAN 上周期发送只带路由信息选项(RIO, RFC 4191)的 RA：不宣告默认路由、不带前缀，' +
			  '只让主机把指定的 IPv6 网段交给本机转发。适合上游不下发前缀、只能 NAT66 的场景。'));

		s = m.section(form.TypedSection, '_status');
		s.anonymous = true;
		s.render = function () {
			var box = E('div', { 'id': 'rioadv-status' }, renderStatus(data[0]));
			if (!polling) {
				polling = true;
				poll.add(function () {
					return callServiceList('rioadv').then(function (res) {
						var el = document.getElementById('rioadv-status');
						if (el)
							dom.content(el, renderStatus(res));
					});
				}, 5);
			}
			return E('div', { 'class': 'cbi-section' }, [ E('h3', _('运行状态')), box ]);
		};

		s = m.section(form.NamedSection, 'main', 'rioadv', _('设置'));
		s.addremove = false;

		o = s.option(form.Flag, 'enabled', _('启用'));
		o.rmempty = false;

		o = s.option(widgets.DeviceSelect, 'interface', _('发送网卡'),
			_('RA 从这块网卡发出，一般是 LAN 的网桥(如 br-lan)'));
		o.noaliases = true;
		o.nocreate = true;
		o.rmempty = false;
		o.default = 'br-lan';

		o = s.option(form.ListValue, 'dhcp_section', _('M/O 标志来源'),
			_('读取该 DHCP 段的 ra_flags，让本服务发的 RA 与 odhcpd 的标志一致，免得主机来回切换 DHCPv6 状态'));
		uci.sections('dhcp', 'dhcp', function (d) {
			o.value(d['.name'], d.interface ? d['.name'] + ' (' + d.interface + ')' : d['.name']);
		});
		o.default = 'lan';

		o = s.option(form.DynamicList, 'route', _('宣告的网段'),
			_('IPv6 网段/前缀长度，例如 2001:250:3402::/48。主机会把去这些网段的流量发给本机，' +
			  '本机需要能转发出去(如开了 NAT66)'));
		o.datatype = 'cidr6';
		o.rmempty = false;

		o = s.option(form.Value, 'interval', _('发送间隔(秒)'));
		o.datatype = 'range(4,1800)';
		o.placeholder = '60';
		o.default = '60';

		o = s.option(form.Value, 'lifetime', _('路由寿命(秒)'),
			_('主机上这条路由的有效期，停发后最多这么久才过期(正常停止服务时会立即撤销)'));
		o.datatype = 'range(1,65535)';
		o.placeholder = '1800';
		o.default = '1800';
		o.validate = function (section_id, value) {
			var iv = +(this.section.formvalue(section_id, 'interval') || 60);
			if (value === '' || +value >= iv * 3)
				return true;
			return _('应不小于发送间隔的 3 倍，否则丢一两个包路由就会过期');
		};

		return m.render();
	}
});
