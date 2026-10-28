'require view';
'require rpc';
'require ui';
'require poll';

var callList = rpc.declare({ object: 'docker-exposure', method: 'list' });
var callSetMode = rpc.declare({ object: 'docker-exposure', method: 'set_mode', params: [ 'name', 'mode' ] });
var callApply = rpc.declare({ object: 'docker-exposure', method: 'apply' });

var STATES = {
	guarded: [ '拦截规则已加载', 'success', '已核对当前入口和所有发布端口的拦截规则。' ],
	allowed: [ '本工具不拦截', 'warning', '是否能从公网访问，还取决于网络和其他防火墙规则。' ],
	unmanaged: [ '由宿主机防火墙管理', 'muted', '使用 host 网络，不由本页控制。' ],
	unpublished: [ '未发布端口', 'muted', '没有 Docker 端口映射，本页无需设置。' ],
	unconfigured: [ '尚未纳入管理', 'danger', '没有对应的容器配置，不能确认已受保护。' ],
	exception: [ '存在端口例外', 'danger', '部分端口在例外列表中，选择阻止 WAN 也不会为它们生成拦截规则。' ],
	partial: [ '部分端口缺少规则', 'danger', '部分发布端口未找到匹配的拦截规则。' ],
	missing: [ '未找到拦截规则', 'danger', '已配置阻止 WAN，但当前规则未覆盖发布端口。' ],
	mismatch: [ '仍有拦截规则', 'danger', '已配置允许 WAN，但当前仍发现拦截规则。' ],
	unknown: [ '状态读取失败', 'danger', '无法读取容器信息，请刷新后重试。' ]
};

var STYLE = `
.de-page{--de-line:var(--hairline,#ddd);--de-muted:var(--text-muted,#777);max-width:1500px;margin:0 auto;padding-bottom:18px}
.de-page *{box-sizing:border-box}.de-page button{font:inherit}.de-top{display:flex;justify-content:space-between;align-items:flex-start;gap:20px;margin:8px 0 24px}.de-eyebrow{font-size:11px;letter-spacing:.16em;color:var(--de-muted);margin-bottom:10px}.de-page h2{font-size:27px;font-weight:650;margin:0 0 10px;line-height:1.3}.de-subtitle{color:var(--de-muted);line-height:1.7;margin:0;max-width:780px}.de-top-actions{display:flex;gap:10px;align-items:center;flex-shrink:0;margin-top:22px}
.de-button{border:1px solid var(--de-line);border-radius:9px;padding:9px 15px;cursor:pointer;background:var(--control-bg,var(--surface,#fff));color:inherit;white-space:nowrap}.de-button:hover{background:var(--hover-faint,#f6f6f6)}.de-button:focus-visible,.de-stat:focus-visible,.de-option:focus-visible{outline:2px solid var(--brand,#c96442);outline-offset:3px}.de-button:disabled{opacity:.45;cursor:default}.de-primary{background:var(--brand,#c96442);border-color:var(--brand,#c96442);color:var(--on-brand,#fff);font-weight:600}.de-primary:hover{background:var(--brand-hover,#b55230)}
.de-overview{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:12px;margin-bottom:18px}.de-stat{text-align:left;border:1px solid var(--de-line);background:var(--surface,#fff);color:inherit;border-radius:12px;padding:17px 20px;cursor:pointer;min-width:0}.de-stat:hover{background:var(--hover-faint,#f5f5f5)}.de-stat.is-active{border-color:var(--brand,#c96442);box-shadow:inset 0 0 0 1px var(--brand,#c96442)}.de-stat-head{display:flex;align-items:center;gap:8px;font-size:13px;color:var(--de-muted)}.de-dot{width:7px;height:7px;display:inline-block;border-radius:50%;background:currentColor;flex-shrink:0}.de-stat strong{font-size:30px;font-weight:600;line-height:1.5;display:block;margin-top:6px}.de-stat small{color:var(--de-muted);font-size:12px}.de-success{color:var(--success,#067647)}.de-warning{color:var(--warning,#995b00)}.de-danger{color:var(--danger,#b42318)}.de-muted{color:var(--de-muted)}
.de-page button.de-stat{display:flex;flex-direction:column;align-items:stretch;justify-content:flex-start;gap:0;white-space:normal}
.de-health{display:flex;gap:10px 20px;flex-wrap:wrap;align-items:center;padding:0 2px 20px;color:var(--de-muted);font-size:12px}.de-health-item{display:inline-flex;gap:7px;align-items:center}.de-health-item b{font-weight:500;color:var(--text,inherit)}.de-alert{border:1px solid var(--danger,#b42318);border-radius:10px;background:var(--danger-surface,#fff3f2);padding:13px 16px;margin-bottom:18px;line-height:1.7}.de-alert strong{display:block}.de-panel{border:1px solid var(--de-line);border-radius:12px;background:var(--surface,#fff);overflow:hidden;margin-bottom:16px}.de-toolbar{padding:17px 20px;display:flex;justify-content:space-between;align-items:center;gap:16px;border-bottom:1px solid var(--de-line)}.de-panel-title{font-weight:600;font-size:15px}.de-counter{margin-left:8px;font-size:12px;color:var(--de-muted);font-weight:400}.de-search{width:260px!important;max-width:100%;border:1px solid var(--de-line)!important;border-radius:8px!important;background:var(--control-bg,#fff)!important;padding:8px 12px!important;font:inherit;color:inherit!important;height:auto!important;min-height:36px}.de-filter-note{font-size:12px;color:var(--de-muted);padding:10px 20px;border-bottom:1px solid var(--de-line);display:flex;align-items:center;gap:12px}.de-text-button{border:0;background:none;color:var(--brand,#c96442);cursor:pointer;padding:0;font:inherit}
.de-table-wrap{overflow-x:auto}.de-table{width:100%;border-collapse:collapse;margin:0;table-layout:fixed}.de-table thead th{text-align:left;padding:12px 20px;color:var(--de-muted);font-size:12px;font-weight:500;background:var(--surface-sunken,#f7f7f7);border-bottom:1px solid var(--de-line)}.de-table th:nth-child(1){width:23%}.de-table th:nth-child(2){width:27%}.de-table th:nth-child(3){width:26%}.de-table th:nth-child(4){width:24%}.de-table td{padding:17px 20px;vertical-align:middle;border-bottom:1px solid var(--de-line)}.de-table tbody tr:last-child td{border-bottom:0}.de-table tr.de-pending{background:var(--brand-subtle,#fff5ee)}.de-name{font-weight:600;overflow-wrap:anywhere;font-size:14px}.de-network{display:inline-block;font-size:10px;line-height:1.6;border:1px solid var(--de-line);border-radius:5px;padding:0 5px;color:var(--de-muted);margin-top:6px;max-width:100%;overflow-wrap:anywhere}.de-port{display:flex;gap:8px;align-items:center;flex-wrap:wrap;font-size:13px;line-height:1.5}.de-port+.de-port{margin-top:9px}.de-port code{background:none;padding:0;color:inherit;font-size:14px}.de-arrow{color:var(--de-muted)}.de-protocol{font-size:10px;color:var(--de-muted);letter-spacing:.04em}.de-binding{font-size:11px;color:var(--de-muted);margin-top:4px;overflow-wrap:anywhere}.de-segment{display:inline-flex;max-width:100%;padding:3px;border:1px solid var(--de-line);border-radius:9px;background:var(--control-bg,var(--surface,#fff));gap:3px}.de-option{font-size:12px!important;padding:7px 11px;border:0;border-radius:6px;background:none;color:var(--de-muted);cursor:pointer;white-space:nowrap}.de-option[aria-pressed="true"]{background:var(--brand-subtle,#f9e7db);color:var(--brand,#c96442);box-shadow:0 1px 3px #0000000a;font-weight:600}.de-option[aria-pressed="true"].de-option-wan{background:var(--warning-surface,#fff3d6);color:var(--warning,#995b00)}.de-option:disabled{cursor:default;opacity:.5}.de-badge{display:inline-flex;gap:6px;align-items:center;font-size:12px;line-height:1.6;padding:3px 8px;border-radius:6px;background:var(--surface-sunken,#f6f6f6)}.de-badge.de-success{background:var(--success-surface,#eaf8f0)}.de-badge.de-warning{background:var(--warning-surface,#fff3d6)}.de-badge.de-danger{background:var(--danger-surface,#fff0ed)}.de-row-help{font-size:11px;color:var(--de-muted);line-height:1.6;margin-top:7px}.de-unsaved{font-size:11px;color:var(--brand,#c96442);margin-top:6px}.de-empty{padding:40px 20px;text-align:center;color:var(--de-muted);line-height:1.8}.de-secondary-list{padding:0 20px}.de-secondary-row{display:flex;align-items:center;justify-content:space-between;gap:16px;padding:15px 0;border-bottom:1px solid var(--de-line)}.de-secondary-row:last-child{border-bottom:0}.de-secondary-name{display:flex;align-items:baseline;flex-wrap:wrap;gap:9px}.de-secondary-name .de-network{margin:0}.de-secondary-note{font-size:12px;color:var(--de-muted);margin-top:5px;line-height:1.6}.de-footer{display:flex;align-items:center;justify-content:space-between;gap:15px;padding:15px 20px;border:1px solid var(--de-line);border-radius:10px;background:var(--surface,#fff);position:sticky;bottom:12px;z-index:5;box-shadow:0 3px 15px #00000008}.de-footer-title{font-size:13px;font-weight:500}.de-footer-note{font-size:11px;color:var(--de-muted);margin-top:5px}.de-footer-actions{display:flex;gap:9px}.de-details{margin:22px 2px 0;color:var(--de-muted);font-size:12px;line-height:1.8}.de-details summary{cursor:pointer;display:list-item}.de-details p{margin:9px 0}.de-details code{font-size:11px;overflow-wrap:anywhere}.de-details-body{padding:3px 0 0 16px}
.de-other-panel>summary{cursor:pointer;list-style:none;border-bottom:0;justify-content:flex-start}.de-other-panel>summary::before{content:'▸';color:var(--de-muted)}.de-other-panel[open]>summary{border-bottom:1px solid var(--de-line)}.de-other-panel[open]>summary::before{content:'▾'}
@media(min-width:1400px){.de-page{padding-top:10px}.de-table td{padding-top:19px;padding-bottom:19px}}
@media(max-width:900px){.de-top{flex-direction:column;gap:12px}.de-top-actions{margin-top:0}.de-table{min-width:820px}.de-overview{gap:8px}.de-stat{padding:13px}.de-stat-head{font-size:12px}.de-table td,.de-table thead th{padding-left:14px;padding-right:14px}}
@media(max-width:540px){.de-page h2{font-size:24px}.de-overview{grid-template-columns:repeat(2,minmax(0,1fr))}.de-stat strong{font-size:27px}.de-toolbar{flex-wrap:wrap;padding:14px}.de-search{width:100%!important}.de-toolbar .de-search{flex-basis:100%}.de-top-actions{width:100%;justify-content:space-between}.de-footer{align-items:flex-start;flex-direction:column;bottom:6px;padding:13px}.de-footer-actions{width:100%}.de-footer-actions button{flex:1}.de-secondary-list{padding:0 14px}.de-secondary-row{align-items:flex-start}.de-health{gap:9px 14px}.de-subtitle{font-size:13px}.de-secondary-row .de-badge{max-width:135px}}
@media(max-width:640px){.de-table{min-width:0;width:100%;display:block}.de-table thead{display:none}.de-table tbody{display:block}.de-table tbody tr{display:block;padding:17px;border-bottom:1px solid var(--de-line)}.de-table tbody tr:last-child{border-bottom:0}.de-table td{display:block;width:100%;padding:0!important;border:0}.de-table td+td{margin-top:13px}.de-table td[data-label]::before{content:attr(data-label);display:block;font-size:11px;color:var(--de-muted);margin-bottom:5px}.de-table td:first-child .de-name{display:inline}.de-table td:first-child .de-network{margin:0 0 0 8px}.de-table td:last-child .de-row-help{display:inline;margin-left:8px}.de-table .de-binding{font-size:11px}.de-option{padding:8px 15px}}
`;

function hasPorts(c) { return (c.ports || []).length > 0; }
function isOther(c) { return c.state === 'unmanaged' || c.state === 'unpublished'; }
function isAttention(c) { return !isOther(c) && c.state !== 'guarded' && c.state !== 'allowed'; }
function badge(text, tone) {
	return E('span', { 'class': 'de-badge de-' + tone }, [ E('span', { 'class': 'de-dot', 'aria-hidden': 'true' }), text ]);
}

return view.extend({
	load: function() { return callList(); },

	render: function(data) {
		this.data = data || {};
		this.pending = {};
		this.filter = 'all';
		this.search = '';
		this.busy = false;
		this.lastUpdated = new Date();
		this.root = E('div', { 'class': 'de-page' });
		this.paint();
		var self = this;
		poll.add(function() {
			if (!self.root.isConnected || self.busy) return Promise.resolve();
			return self.refresh();
		}, 15);
		return this.root;
	},

	refresh: function() {
		var self = this;
		if (this.refreshing || this.busy) return Promise.resolve();
		this.refreshing = true;
		this.paint();
		return callList().then(function(data) {
			if (!data || !data.status) throw new Error('返回的状态不完整');
			self.data = data;
			self.lastUpdated = new Date();
			self.refreshError = '';
		}).catch(function(err) {
			self.refreshError = '状态更新失败，以下为上次读取的结果：' + err.message;
		}).then(function() { self.refreshing = false; self.paint(); });
	},

	choose: function(c, mode) {
		if (this.busy) return;
		if (c.mode === mode) delete this.pending[c.name];
		else this.pending[c.name] = mode;
		this.paint();
	},

	handleApply: function() {
		var self = this;
		var changes = Object.keys(this.pending).map(function(name) { return { name: name, mode: self.pending[name] }; });
		if (!changes.length || this.busy || this.refreshing) return Promise.resolve();
		this.busy = true;
		this.paint();
		// Serialize UCI writes; keep all pending changes on failure for retry.
		return changes.reduce(function(chain, change) {
			return chain.then(function() {
				return callSetMode(change.name, change.mode).then(function(result) {
					if (!result || result.ok !== true) throw new Error(change.name + ' 保存失败：' + ((result && result.error) || '请重试'));
				});
			});
		}, Promise.resolve()).then(function() {
			return callApply();
		}).then(function(result) {
			if (!result || result.ok !== true) throw new Error('规则应用失败，已保存的配置可能尚未生效，请重试');
			return callList();
		}).then(function(data) {
			if (!data || !data.status) throw new Error('规则已提交，但读取结果失败，请刷新核对');
			self.data = data;
			self.pending = {};
			self.lastUpdated = new Date();
			self.refreshError = '';
			var issues = (data.containers || []).filter(isAttention);
			ui.addNotification(null, E('p', {}, issues.length ? '设置已提交，仍有 ' + issues.length + ' 个容器需要检查，请查看当前规则状态。' : '设置已应用，当前规则状态已重新读取。'), issues.length ? 'warning' : 'info');
		}).catch(function(err) {
			self.refreshError = err.message;
			ui.addNotification(null, E('p', {}, err.message), 'danger');
		}).then(function() { self.busy = false; self.paint(); });
	},

	portView: function(c) {
		var groups = {};
		(c.bindings || []).forEach(function(b) {
			var key = b.host_port + '/' + b.container_port + '/' + b.protocol;
			if (!groups[key]) groups[key] = { port: b.host_port, target: b.container_port, protocol: b.protocol, addresses: [] };
			if (groups[key].addresses.indexOf(b.host_ip) < 0) groups[key].addresses.push(b.host_ip);
		});
		if (!Object.keys(groups).length) return E('span', { 'class': 'de-muted' }, (c.ports || []).join(', ') || '未发布端口');
		return E('div', {}, Object.keys(groups).map(function(key) {
			var g = groups[key];
			var addresses = g.addresses.map(function(ip) {
				if (ip === '0.0.0.0') return '全部 IPv4 地址';
				if (ip === '::') return '全部 IPv6 地址';
				return ip || '默认地址';
			});
			return E('div', { 'class': 'de-port' }, [
				E('div', {}, [ E('code', {}, g.port), E('span', { 'class': 'de-arrow', 'aria-label': '映射到' }, ' → '), E('code', {}, g.target), ' ', E('span', { 'class': 'de-protocol' }, g.protocol.toUpperCase()), E('div', { 'class': 'de-binding' }, addresses.join(' · ')) ])
			]);
		}));
	},

	row: function(c) {
		var self = this;
		var pending = Object.prototype.hasOwnProperty.call(this.pending, c.name);
		var selected = pending ? this.pending[c.name] : c.mode;
		var state = STATES[c.state] || STATES.unknown;
		var editable = c.configured === true && hasPorts(c) && c.state !== 'unknown';
		var selection = editable ? E('div', { 'class': 'de-segment', 'role': 'group', 'aria-label': c.name + ' 公网访问' }, [ 'lan', 'wan' ].map(function(mode) {
			return E('button', {
				'type': 'button', 'class': 'de-option de-option-' + mode,
				'aria-pressed': selected === mode ? 'true' : 'false',
				'data-focus-key': c.name + '-' + mode,
				'disabled': self.busy || null,
				'click': function() { self.choose(c, mode); }
			}, mode === 'lan' ? '阻止 WAN' : '允许 WAN');
		})) : E('span', { 'class': 'de-muted' }, '暂不可设置');
		var help;
		if (c.state === 'guarded') help = c.covered_ports + '/' + c.total_ports + ' 个端口已覆盖 · 累计丢弃 ' + Number(c.blocked_packets || 0).toLocaleString() + ' 包';
		else help = state[2];
		var accessChildren = [ selection ];
		if (pending)
			accessChildren.push(E('div', { 'class': 'de-unsaved' }, '待应用 · 当前已保存：' + (c.mode === 'wan' ? '允许 WAN' : '阻止 WAN')));
		return E('tr', { 'class': pending ? 'de-pending' : '' }, [
			E('td', {}, [ E('div', { 'class': 'de-name' }, c.name), E('span', { 'class': 'de-network' }, c.network || '未知网络') ]),
			E('td', { 'data-label': '宿主机端口 → 容器端口' }, this.portView(c)),
			E('td', { 'data-label': '公网访问' }, accessChildren),
			E('td', { 'data-label': '当前规则状态' }, [ badge(state[0], state[1]), E('div', { 'class': 'de-row-help' }, help) ])
		]);
	},

	paint: function() {
		var self = this;
		var active = document.activeElement;
		var focusKey = active && active.getAttribute('data-focus-key');
		var cursor = active && active.tagName === 'INPUT' ? active.selectionStart : null;
		var containers = this.data.containers || [];
		var status = this.data.status || {};
		var count = Object.keys(this.pending).length;
		var groups = [
			{ key: 'guarded', title: '阻止 WAN', count: containers.filter(function(c) { return c.state === 'guarded'; }).length, note: '拦截规则已核对', tone: 'success' },
			{ key: 'allowed', title: '本工具放行', count: containers.filter(function(c) { return c.state === 'allowed'; }).length, note: '不代表公网一定可达', tone: 'warning' },
			{ key: 'attention', title: '需要处理', count: containers.filter(isAttention).length, note: '缺少规则或存在例外', tone: 'danger' },
			{ key: 'other', title: '其他容器', count: containers.filter(isOther).length, note: 'host 网络或未发布端口', tone: 'muted' }
		];
		var filtered = containers.filter(function(c) {
			if (self.search && (c.name + ' ' + (c.ports || []).join(' ')).toLowerCase().indexOf(self.search.toLowerCase()) < 0) return false;
			if (self.filter === 'attention') return isAttention(c);
			if (self.filter === 'other') return isOther(c);
			return self.filter === 'all' || c.state === self.filter;
		});
		var managed = filtered.filter(function(c) { return !isOther(c); });
		var others = filtered.filter(isOther);
		var healthy = status.docker_api === 'ok' && status.service === 'running' && status.chain_valid === true;
		var alerts = [];
		if (this.refreshError) alerts.push(this.refreshError);
		if (status.docker_api !== 'ok') alerts.push('Docker 信息读取异常，容器列表可能不完整。');
		if (status.service !== 'running') alerts.push('自动同步服务未运行，容器变化可能无法及时更新。');
		if (status.chain_valid !== true) alerts.push('未找到有效的拦截链，请检查防护服务。');
		var timestamp = this.lastUpdated.toLocaleTimeString('zh-CN', { hour12: false });
		var children = [
			E('style', {}, STYLE),
			E('div', { 'class': 'de-top' }, [
				E('div', {}, [ E('div', { 'class': 'de-eyebrow' }, 'DOCKER / NETWORK ACCESS'), E('h2', {}, 'Docker 端口访问控制'), E('p', { 'class': 'de-subtitle' }, '选择哪些容器阻止 WAN 直连，哪些交给现有网络规则。选择后点击“应用修改”，局域网访问不变。') ]),
				E('div', { 'class': 'de-top-actions' }, [ badge(healthy ? '自动同步运行中' : '运行状态需检查', healthy ? 'success' : 'danger'), E('button', { 'type': 'button', 'class': 'de-button', 'disabled': (this.busy || this.refreshing) || null, 'data-focus-key': 'refresh', 'click': function() { return self.refresh(); } }, this.refreshing ? '正在刷新…' : '刷新状态') ])
			]),
			E('div', { 'class': 'de-overview' }, groups.map(function(g) {
				return E('button', { 'type': 'button', 'class': 'de-stat' + (self.filter === g.key ? ' is-active' : ''), 'aria-pressed': self.filter === g.key ? 'true' : 'false', 'data-focus-key': 'filter-' + g.key, 'click': function() { self.filter = self.filter === g.key ? 'all' : g.key; self.paint(); } }, [ E('span', { 'class': 'de-stat-head' }, [ E('span', { 'class': 'de-dot de-' + g.tone, 'aria-hidden': 'true' }), g.title ]), E('strong', {}, String(g.count)), E('small', {}, g.note) ]);
			})),
			E('div', { 'class': 'de-health' }, [ E('span', { 'class': 'de-health-item' }, [ '拦截入口 ', E('b', {}, status.wan_iface || '未知') ]), E('span', { 'class': 'de-health-item' }, [ '运行规则 ', E('b', {}, String(status.runtime_rules == null ? '—' : status.runtime_rules) + ' 条') ]), E('span', { 'class': 'de-health-item' }, [ '重启保留 ', E('b', {}, String(status.persist_rules == null ? '—' : status.persist_rules) + ' 条') ]), E('span', { 'class': 'de-health-item' }, '更新于 ' + timestamp + ' · 每 15 秒刷新') ])
		];
		if (alerts.length) children.push(E('div', { 'class': 'de-alert', 'role': 'alert' }, [ E('strong', {}, '请检查当前状态'), E('div', {}, alerts.join(' ')) ]));
		var panel = [ E('div', { 'class': 'de-toolbar' }, [ E('div', { 'class': 'de-panel-title' }, [ '端口访问设置', E('span', { 'class': 'de-counter' }, containers.filter(function(c) { return !isOther(c); }).length + ' 个容器') ]), E('input', { 'class': 'de-search', 'type': 'search', 'placeholder': '搜索容器或端口…', 'aria-label': '搜索容器或端口', 'value': this.search, 'data-focus-key': 'search', 'input': function(ev) { self.search = ev.target.value; self.paint(); } }) ]) ];
		if (this.filter !== 'all' || this.search) panel.push(E('div', { 'class': 'de-filter-note' }, [ '当前显示 ' + filtered.length + ' / ' + containers.length + ' 个容器', E('button', { 'type': 'button', 'class': 'de-text-button', 'click': function() { self.filter = 'all'; self.search = ''; self.paint(); } }, '显示全部') ]));
		panel.push(managed.length ? E('div', { 'class': 'de-table-wrap' }, E('table', { 'class': 'de-table' }, [ E('thead', {}, E('tr', {}, [ '容器', '端口映射 · 宿主机 → 容器', '公网访问', '当前规则状态' ].map(function(label) { return E('th', { 'scope': 'col' }, label); }))), E('tbody', {}, managed.map(function(c) { return self.row(c); })) ])) : E('div', { 'class': 'de-empty' }, containers.length ? '没有符合条件的端口设置' : (status.docker_api === 'ok' ? '当前没有运行中的容器' : '无法读取容器列表，请检查 Docker 服务')));
		children.push(E('section', { 'class': 'de-panel', 'aria-label': '端口访问设置' }, panel));
		if (others.length) {
			var otherPanel = E('details', { 'class': 'de-panel de-other-panel', 'aria-label': '其他容器' }, [ E('summary', { 'class': 'de-toolbar' }, E('div', { 'class': 'de-panel-title' }, [ '其他容器', E('span', { 'class': 'de-counter' }, others.length + ' 个 · 本页无需设置或不提供控制') ])), E('div', { 'class': 'de-secondary-list' }, others.map(function(c) {
			var state = STATES[c.state] || STATES.unknown;
			return E('div', { 'class': 'de-secondary-row' }, [ E('div', {}, [ E('div', { 'class': 'de-secondary-name' }, [ E('span', { 'class': 'de-name' }, c.name), E('span', { 'class': 'de-network' }, c.network) ]), E('div', { 'class': 'de-secondary-note' }, state[2]) ]), badge(c.state === 'unpublished' ? '未发布端口' : '本页不管理', 'muted') ]);
		})) ]);
			otherPanel.open = this.filter === 'other' || !!this.othersOpen;
			otherPanel.addEventListener('toggle', function() { if (otherPanel.isConnected) self.othersOpen = otherPanel.open; });
			children.push(otherPanel);
		}
		children.push(E('div', { 'class': 'de-footer' }, [ E('div', { 'aria-live': 'polite' }, [ E('div', { 'class': 'de-footer-title' }, this.busy ? '正在保存配置并更新规则…' : (count ? count + ' 个容器的修改待应用' : '没有待应用的修改')), E('div', { 'class': 'de-footer-note' }, count ? '表中的规则状态仍是当前实际状态，应用后会重新核对。' : '上方统计依据当前规则，不受尚未应用的选择影响。') ]), E('div', { 'class': 'de-footer-actions' }, [ E('button', { 'type': 'button', 'class': 'de-button', 'disabled': (!count || this.busy) || null, 'click': function() { self.pending = {}; self.paint(); } }, '撤销选择'), E('button', { 'type': 'button', 'class': 'de-button de-primary', 'disabled': (!count || this.busy || this.refreshing) || null, 'click': function() { return self.handleApply(); } }, this.busy ? '正在应用…' : '应用修改' + (count ? '（' + count + '）' : '')) ]) ]));
		var details = E('details', { 'class': 'de-details' }, [ E('summary', {}, '访问说明与端口例外'), E('div', { 'class': 'de-details-body' }, [ E('p', {}, '阻止 WAN：拦截从上方指定入口进入的发布端口流量。其他网络来源是否可访问，仍由现有防火墙决定。允许 WAN：本工具不生成拦截规则，不保证公网一定可达。'), E('p', {}, '端口例外：' + (this.data.whitelist || '无') + '。容器使用这些宿主机端口时会跳过拦截，并在表中提示。'), E('p', {}, '“拦截规则已加载”表示已核对当前规则；公网 IPv4 / IPv6 是否确实阻断，需要结合外部访问测试确认。host 网络和未发布端口的容器不在本页控制范围。') ]) ]);
		details.open = !!this.detailsOpen;
		details.addEventListener('toggle', function() { if (details.isConnected) self.detailsOpen = details.open; });
		children.push(details);
		while (this.root.firstChild) this.root.removeChild(this.root.firstChild);
		children.forEach(function(child) { self.root.appendChild(child); });
		if (focusKey) Array.prototype.some.call(this.root.querySelectorAll('[data-focus-key]'), function(el) {
			if (el.getAttribute('data-focus-key') !== focusKey) return false;
			el.focus({ preventScroll: true });
			if (cursor != null && el.setSelectionRange) { try { el.setSelectionRange(cursor, cursor); } catch (err) {} }
			return true;
		});
	},

	handleSave: null,
	handleSaveApply: null,
	handleReset: null
});
