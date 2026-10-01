'use strict';
'require view';
'require rpc';
'require ui';
'require form';
'require uci';
'require poll';
'require dom';

const callList = rpc.declare({ object: 'overlay-restore', method: 'list', expect: { '': {} } });
const callPrepare = rpc.declare({ object: 'overlay-restore', method: 'prepare', expect: { '': {} } });
const callStatus = rpc.declare({ object: 'overlay-restore', method: 'status', params: [ 'id' ], expect: { '': {} } });
const callApply = rpc.declare({ object: 'overlay-restore', method: 'apply', params: [ 'id', 'confirmation' ], expect: { '': {} } });
const callRetry = rpc.declare({ object: 'overlay-restore', method: 'retry', params: [ 'id' ], expect: { '': {} } });
const callCommit = rpc.declare({ object: 'uci', method: 'commit', params: [ 'config' ], expect: { '': {} } });

const labels = {
    validating: '正在检查备份', ready: '等待确认', queued: '等待执行', preparing_packages: '正在准备 DNS / 代理软件', applying: '正在迁移配置',
    awaiting_reboot: '配置已迁移，等待重启', installing: '正在安装软件并修复服务',
    complete: '恢复完成', complete_with_warnings: '恢复完成，部分项目有警告',
    failed_validation: '备份检查失败', failed_prepare: '网络软件准备失败，配置尚未修改', failed_apply: '配置迁移失败', failed_packages: '软件恢复尚未完成'
};

function checked(result) {
    if (result.error)
        throw new Error(result.error);
    return result;
}

return view.extend({
    taskId: null,
    load: function() {
        return Promise.all([ uci.load('overlay_restore'), callList().then(checked) ]);
    },

    inspect: function(ev) {
        return this.map.save()
            .then(function() { return callCommit('overlay_restore'); })
            .then(function() { return ui.uploadFile('/tmp/overlay-restore-upload.tar.gz'); })
            .then(function() { return callPrepare().then(checked); })
            .then(L.bind(function(result) { this.taskId = result.id; return this.refresh(); }, this))
            .catch(function(error) { ui.addNotification(null, E('p', error.message)); });
    },

    confirm: function(task) {
        const settings = task.plan.settings;
        ui.showModal('确认恢复计划', [
            E('p', '将覆盖预览中的配置和自定义文件，并在重启后恢复所选软件包。'),
            E('p', '当前内核、包管理状态、软件源、公钥和恢复工具会保留。'),
            settings.restore_credentials ? E('p', { 'class': 'alert-message warning' }, '备份中的账号及 SSH 凭据会恢复，重连后可能需要使用备份中的密码登录。') : '',
            task.plan.lan_ip ? E('p', '备份中的 LAN 地址：' + task.plan.lan_ip) : '',
            E('p', settings.reboot ? '配置迁移完成后会自动重启。' : '配置迁移完成后需要手动重启。'),
            E('div', { 'class': 'right' }, [
                E('button', { 'class': 'btn', 'click': ui.hideModal }, '取消'), ' ',
                E('button', {
                    'class': 'btn cbi-button-action important',
                    'click': ui.createHandlerFn(this, function() {
                        return callApply(task.id, task.id).then(checked).then(L.bind(function() {
                            ui.hideModal();
                            return this.refresh();
                        }, this)).catch(function(error) { ui.addNotification(null, E('p', error.message)); });
                    })
                }, '执行恢复')
            ])
        ]);
    },

    renderTask: function(task) {
        const body = [ E('h3', labels[task.status] || task.status), E('p', '任务：' + task.id) ];
        if (task.error)
            body.push(E('p', { 'class': 'alert-message danger' }, task.error));
        (task.warnings || []).forEach(function(warning) { body.push(E('p', { 'class': 'alert-message warning' }, warning)); });
        if (task.plan) {
            const plan = task.plan, settings = plan.settings;
            body.push(E('p', '备份 SHA256：' + plan.sha256));
            body.push(E('p', '将迁移 ' + plan.file_count + ' 个文件，已写入 ' + task.completed_count + ' 个。'));
            body.push(E('p', '自用软件源：' + plan.myfeed_repo));
            body.push(E('p', settings.keep_current_extroot ? '保留当前系统的 extroot 挂载配置。' : '使用备份中的完整 fstab。'));
            body.push(E('details', {}, [
                E('summary', '待恢复文件（最多显示 200 项）'),
                E('pre', { 'style': 'max-height:260px;overflow:auto' }, plan.files.map(function(file) { return file.path; }).join('\n'))
            ]));
            body.push(E('details', {}, [
                E('summary', '软件安装及移除计划'),
                E('pre', '当前源安装：\n' + settings.install_packages.join(' ') + '\n\nmyfeed 安装：\n' + settings.myfeed_packages.join(' ') +
                    '\n\n可选安装：\n' + settings.optional_packages.join(' ') + '\n\n移除：\n' + settings.remove_packages.join(' '))
            ]));
            body.push(E('details', {}, [ E('summary', '跳过的备份内容'), E('pre', JSON.stringify(plan.skipped, null, 2)) ]));
        }
        if (task.status == 'ready' && L.hasViewPermission())
            body.push(E('button', { 'class': 'btn cbi-button-action', 'click': L.bind(this.confirm, this, task) }, '确认恢复计划'));
        if ((task.status == 'failed_prepare' || task.status == 'failed_packages' || task.status == 'complete_with_warnings') && L.hasViewPermission())
            body.push(E('button', { 'class': 'btn', 'click': ui.createHandlerFn(this, function() {
                return callRetry(task.id).then(checked).then(L.bind(this.refresh, this)).catch(function(error) {
                    ui.addNotification(null, E('p', error.message));
                });
            }) }, '重试软件及服务修复'));
        const rows = Object.keys(task.packages || {}).map(function(name) {
            const item = task.packages[name];
            return E('tr', { 'class': 'tr' }, [ E('td', { 'class': 'td' }, name), E('td', { 'class': 'td' }, item.operation),
                E('td', { 'class': 'td' }, item.status), E('td', { 'class': 'td' }, item.message || '') ]);
        });
        if (rows.length)
            body.push(E('table', { 'class': 'table' }, [ E('tr', { 'class': 'tr table-titles' },
                [ '软件包', '操作', '结果', '说明' ].map(function(title) { return E('th', { 'class': 'th' }, title); })) ].concat(rows)));
        body.push(E('details', { 'open': '' }, [ E('summary', '执行日志'),
            E('pre', { 'style': 'max-height:320px;overflow:auto;white-space:pre-wrap' }, task.log || '等待执行…') ]));
        return E('div', { 'class': 'cbi-section' }, body);
    },

    refresh: function() {
        return callList().then(checked).then(L.bind(function(result) {
            dom.content(this.history, [ E('h3', '最近的恢复任务') ].concat((result.tasks || []).map(L.bind(function(task) {
                return E('button', { 'class': 'btn', 'style': 'margin:4px', 'click': L.bind(function() {
                    this.taskId = task.id;
                    return this.refresh();
                }, this) }, labels[task.status] + ' · ' + task.id.slice(0, 8));
            }, this))));
            if (!this.taskId && result.tasks && result.tasks.length)
                this.taskId = result.tasks[0].id;
            return this.taskId ? callStatus(this.taskId).then(checked) : null;
        }, this)).then(L.bind(function(task) {
            if (!task)
                return;
            dom.content(this.status, this.renderTask(task));
            this.upload.disabled = !L.hasViewPermission() || [ 'validating', 'queued', 'preparing_packages', 'applying', 'awaiting_reboot', 'installing' ].indexOf(task.status) >= 0;
            if (task.status == 'awaiting_reboot' && task.plan.settings.reboot && !task.reboot_failed && !this.reconnecting) {
                this.reconnecting = true;
                ui.showModal('正在等待重启', [ E('p', { 'class': 'spinning' }, '重启后重新登录此页面，可查看软件包恢复结果。') ]);
                const addresses = [ window.location.host ];
                if (task.plan.lan_ip)
                    addresses.push(task.plan.lan_ip);
                ui.awaitReconnect.apply(ui, addresses);
            }
        }, this));
    },

    render: function(data) {
        const map = this.map = new form.Map('overlay_restore', '备份迁移恢复',
            '上传 overlay 或 sysupgrade 备份，先检查恢复计划，再迁移配置和自定义文件。软件包会在重启后从当前软件源重新安装。');
        map.readonly = !L.hasViewPermission();
        const section = map.section(form.NamedSection, 'main', 'restore');
        section.tab('general', '恢复选项');
        section.tab('packages', '软件包');
        section.tab('services', '服务修复');
        section.tab('limits', '备份限制');
        [ [ 'keep_current_extroot', '保留当前 extroot' ], [ 'keep_network', '保留当前网络、防火墙和 DHCP 配置' ],
          [ 'restore_credentials', '恢复备份中的账号及 SSH 凭据' ], [ 'reboot', '迁移完成后自动重启' ] ].forEach(function(item) {
            const option = section.taboption('general', form.Flag, item[0], item[1]);
            option.rmempty = false;
        });
        [ [ 'install_packages', '从当前源安装' ], [ 'myfeed_packages', '从 myfeed 安装' ], [ 'optional_packages', '从 myfeed 尝试安装' ],
          [ 'remove_packages', '移除预装 LuCI 软件包' ] ].forEach(function(item) {
            section.taboption('packages', form.DynamicList, item[0], item[1]);
        });
        section.taboption('packages', form.Value, 'myfeed_repo', '默认 myfeed 地址', '系统已有 00-myfeed.list 时优先使用其中的地址。');
        section.taboption('packages', form.Value, 'myfeed_key_url', 'myfeed 公钥地址');
        section.taboption('services', form.Flag, 'iptv_enable', '恢复 IPTV Refresh').rmempty = false;
        [ [ 'iptv_repo_root', 'IPTV 数据目录' ], [ 'iptv_refresh_iface', 'IPTV 网络接口' ], [ 'iptv_refresh_host', '刷新服务监听地址' ],
          [ 'iptv_refresh_port', '刷新服务端口' ], [ 'iptv_public_url', 'IPTV 对外地址' ], [ 'ha_config_root', 'Home Assistant 配置目录' ] ].forEach(function(item) {
            section.taboption('services', form.Value, item[0], item[1]);
        });
        section.taboption('services', form.DynamicList, 'iptv_refresh_allow_ips', '刷新服务允许的 IP');
        section.taboption('services', form.DynamicList, 'iptv_nginx_allow_ips', '代理允许的 IP / 网段');
        const token = section.taboption('services', form.Value, 'iptv_refresh_token', '刷新服务令牌', '留空时生成随机令牌。');
        token.password = true;
        [ [ 'max_upload_mb', '上传大小上限（MiB）', 'range(1,1024)' ], [ 'max_expanded_mb', '展开大小上限（MiB）', 'range(8,4096)' ] ].forEach(function(item) {
            section.taboption('limits', form.Value, item[0], item[1]).datatype = item[2];
        });
        return map.render().then(L.bind(function(node) {
            this.status = E('div');
            this.history = E('div', { 'class': 'cbi-section' });
            this.upload = E('button', { 'class': 'btn cbi-button-action', 'disabled': !L.hasViewPermission(),
                'click': ui.createHandlerFn(this, 'inspect') }, '上传并检查备份');
            this.taskId = data[1].tasks && data[1].tasks.length ? data[1].tasks[0].id : null;
            poll.add(L.bind(function() { return this.refresh().catch(function() {}); }, this), 3);
            return E('div', {}, [ node, E('div', { 'class': 'cbi-section' }, [ this.upload ]), this.status, this.history ]);
        }, this));
    },

    handleSaveApply: null,
    handleSave: null,
    handleReset: null
});
