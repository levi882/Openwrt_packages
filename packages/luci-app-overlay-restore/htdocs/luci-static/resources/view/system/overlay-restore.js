'use strict';
'require view';
'require rpc';
'require ui';
'require form';
'require uci';
'require poll';
'require dom';
'require fs';

const callList = rpc.declare({ object: 'overlay-restore', method: 'list', expect: { '': {} } });
const callDevices = rpc.declare({ object: 'overlay-restore', method: 'devices', expect: { '': {} } });
const callPrepare = rpc.declare({ object: 'overlay-restore', method: 'prepare', params: [ 'path' ], expect: { '': {} } });
const callStatus = rpc.declare({ object: 'overlay-restore', method: 'status', params: [ 'id' ], expect: { '': {} } });
const callApply = rpc.declare({ object: 'overlay-restore', method: 'apply', params: [ 'id', 'confirmation' ], expect: { '': {} } });
const callRetry = rpc.declare({ object: 'overlay-restore', method: 'retry', params: [ 'id' ], expect: { '': {} } });
const callUsage = rpc.declare({ object: 'overlay-restore', method: 'usage', expect: { '': {} } });
const callCleanup = rpc.declare({ object: 'overlay-restore', method: 'cleanup', expect: { '': {} } });
const callRemove = rpc.declare({ object: 'overlay-restore', method: 'remove', params: [ 'id', 'confirmation' ], expect: { '': {} } });
const callRollback = rpc.declare({ object: 'overlay-restore', method: 'rollback', params: [ 'id', 'confirmation' ], expect: { '': {} } });
const callDiscardOverlay = rpc.declare({ object: 'overlay-restore', method: 'discard_overlay', params: [ 'id', 'confirmation' ], expect: { '': {} } });
const callCommit = rpc.declare({ object: 'uci', method: 'commit', params: [ 'config' ], expect: { '': {} } });

const labels = {
    validating: '正在检查备份', ready: '等待确认', queued: '等待执行', preparing_packages: '正在准备 DNS / 代理软件', applying: '正在迁移配置',
    awaiting_reboot: '配置已迁移，等待重启', installing: '正在安装软件并修复服务',
    queued_clean: '等待准备干净 overlay', cleaning_overlay: '正在准备干净 overlay',
    awaiting_clean_boot: '正在切换干净 overlay，等待重启', rolling_back: '正在准备回退 overlay',
    awaiting_rollback_boot: '正在回退 overlay，等待重启', rolled_back: '已回退到清理前的环境',
    complete: '恢复完成', complete_with_warnings: '恢复完成，部分项目有警告',
    failed_validation: '备份检查失败', failed_prepare: '网络软件准备失败，配置尚未修改', failed_clean: '干净 overlay 准备或切换失败，当前环境保留',
    failed_apply: '配置迁移失败', failed_packages: '软件恢复尚未完成'
};

function checked(result) {
    // A failed task carries its error alongside id/status for display.
    // RPC failures only carry an error and must reject the call.
    if (result.error && !(typeof result.id == 'string' && typeof result.status == 'string'))
        throw new Error(result.error);
    return result;
}

function uploadDisabled(task) {
    return !L.hasViewPermission() || (!!task &&
        [ 'validating', 'queued', 'queued_clean', 'cleaning_overlay', 'awaiting_clean_boot', 'rolling_back',
          'awaiting_rollback_boot', 'preparing_packages', 'applying', 'awaiting_reboot', 'installing' ].indexOf(task.status) >= 0);
}

function formatSize(bytes) {
    const units = [ 'B', 'KiB', 'MiB', 'GiB' ];
    let value = Number(bytes || 0), unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
        value /= 1024;
        unit++;
    }
    return value.toFixed(unit ? 1 : 0) + ' ' + units[unit];
}

function openQuickFile(options) {
    if (!options.enabled())
        return Promise.resolve();
    const initial = options.value || '/';
    const directory = options.directory ? initial : initial.replace(/\/[^/]*$/, '') || '/';
    const page = new URL(L.url('quickfile'), window.location.origin);
    page.searchParams.set('path', directory);
    const controller = new AbortController();
    const panel = E('div', {}, E('p', { 'class': 'spinning' }, '正在打开 QuickFile…'));
    const notice = E('p', { 'class': 'alert-message warning', 'style': 'display:none' });
    let frame = null, closed = false, selecting = false, detachPicker = function() {};
    const close = function() {
        closed = true;
        controller.abort();
        detachPicker();
        if (frame)
            frame.remove();
        ui.hideModal();
    };
    const currentDirectory = function() {
        const location = new URL(frame.contentWindow.location.href);
        if (location.origin != page.origin || location.pathname.replace(/\/$/, '') != page.pathname)
            throw new Error('请先在 QuickFile 中打开目标目录。');
        let path = location.searchParams.get('path') || '/';
        if (path == '.')
            path = '/';
        else if (path.indexOf('/') != 0)
            path = '/' + path.replace(/^\.\//, '');
        return path;
    };
    const choose = function(getPath) {
        if (closed || selecting || !options.enabled())
            return Promise.resolve();
        selecting = true;
        notice.style.display = 'none';
        let path;
        return Promise.resolve().then(function() {
            path = getPath();
            if (path.indexOf('/') != 0 || /[\x00-\x1f\x7f]/.test(path) || path.split('/').indexOf('..') >= 0)
                throw new Error('请使用路由器上的绝对路径。');
            if (!options.directory && !/\.(tar\.gz|tgz)$/.test(path))
                throw new Error('请选择 .tar.gz / .tgz 备份文件。');
            return fs.stat(path);
        }).then(function(stat) {
            if (stat.type != (options.directory ? 'directory' : 'file'))
                throw new Error(options.directory ? '请选择已有目录。' : '请选择已有的普通备份文件。');
            if (!closed && options.enabled()) {
                options.select(path);
                close();
            }
        }).catch(function(error) {
            if (!closed) {
                dom.content(notice, error.message);
                notice.style.display = '';
            }
        }).finally(function() { selecting = false; });
    };
    const select = options.directory ? E('button', {
        'class': 'btn cbi-button-action', 'disabled': true,
        'click': ui.createHandlerFn(null, function() { return choose(currentDirectory); })
    }, '使用当前目录') : null;
    const attachPicker = function() {
        detachPicker();
        if (closed)
            return;
        if (select)
            select.disabled = true;
        try {
            currentDirectory();
            if (select) {
                select.disabled = false;
                return;
            }
            const frameDocument = frame.contentDocument;
            const click = function(ev) {
                if (ev.button != 0 || closed || !options.enabled())
                    return;
                const target = ev.target.closest ? ev.target : ev.target.parentElement;
                if (!target)
                    return;
                const entry = target.closest('[data-swipe-name]');
                if (!entry || target.closest('button, a, textarea, select, .qf-col-actions, .qf-col-swipe') ||
                    (target.closest('input') && !target.matches('input[type="checkbox"]')) ||
                    entry.querySelector('.fa-folder, .fa-folder-open'))
                    return;
                const name = entry.getAttribute('data-swipe-name');
                if (!name || name.indexOf('/') >= 0 || !/\.(tar\.gz|tgz)$/.test(name))
                    return;
                // Handle selection before QuickFile's archive preview or multi-select handlers.
                ev.preventDefault();
                ev.stopImmediatePropagation();
                choose(function() { return currentDirectory().replace(/\/$/, '') + '/' + name; });
            };
            frameDocument.addEventListener('click', click, true);
            detachPicker = function() { frameDocument.removeEventListener('click', click, true); };
        }
        catch (error) {
            dom.content(notice, error.message);
            notice.style.display = '';
        }
    };
    ui.showModal(options.directory ? 'QuickFile · 选择目录' : 'QuickFile · 选择备份', [
        E('style', '.modal.overlay-restore-quickfile { width:calc(100vw - 32px);max-width:1280px;box-sizing:border-box }'),
        E('p', options.directory ? '在 QuickFile 中打开目标目录，再点击「使用当前目录」。' :
            '点击 .tar.gz / .tgz 备份的文件名、文件行或勾选框即可选择；双击目录进入。'),
        panel,
        notice,
        E('div', { 'class': 'right' }, [
            E('button', { 'class': 'btn', 'click': close }, '关闭'), ' ',
            E('a', { 'class': 'btn', 'href': page.href, 'target': '_blank', 'rel': 'noopener noreferrer' }, '在新窗口打开'), ' ', select || ''
        ])
    ], 'overlay-restore-quickfile');
    const timeout = window.setTimeout(function() { controller.abort(); }, 8000);
    return fetch(page.href, { credentials: 'same-origin', signal: controller.signal }).then(function(response) {
        if (!response.ok || new URL(response.url).origin != page.origin)
            throw new Error('QuickFile 页面不可用');
        return response.text();
    }).then(function(html) {
        if (!/window\.API_PREFIX\s*=/.test(html))
            throw new Error('QuickFile 页面不可用');
        if (closed || !panel.isConnected)
            return;
        frame = E('iframe', {
            'src': page.href, 'title': 'QuickFile 文件管理',
            'style': 'display:block;width:100%;height:60vh;min-height:240px;border:0;border-radius:6px'
        });
        frame.addEventListener('load', attachPicker);
        dom.content(panel, frame);
    }).catch(function() {
        if (!closed && panel.isConnected)
            dom.content(panel, E('p', { 'class': 'alert-message warning' },
                'QuickFile 页面暂不可用。请先安装并启动 luci-app-quickfile，确认 QuickFile 的独立页面能够正常打开。'));
    }).finally(function() { window.clearTimeout(timeout); });
}

const DirectoryPath = form.Value.extend({
    renderWidget: function(sectionId, optionIndex, cfgvalue) {
        return Promise.resolve(this.super('renderWidget', [ sectionId, optionIndex, cfgvalue ])).then(L.bind(function(inputNode) {
            const select = L.bind(function(path) {
                const input = this.getUIElement(sectionId);
                input.setValue(path);
                input.triggerValidation();
                input.node.dispatchEvent(new CustomEvent('widget-change', { bubbles: true }));
            }, this);
            const quickfile = E('button', {
                'class': 'btn', 'disabled': this.map.readonly || null,
                'click': ui.createHandlerFn(this, function() {
                    return openQuickFile({
                        directory: true, value: this.formvalue(sectionId),
                        enabled: L.bind(function() { return !this.map.readonly; }, this), select: select
                    });
                })
            }, '打开 QuickFile 选择目录');
            return E('div', {}, [ inputNode, E('div', { 'style': 'margin-top:8px' }, quickfile) ]);
        }, this));
    }
});

return view.extend({
    taskId: null,
    taskSnapshot: null,
    historySnapshot: null,
    refreshRevision: 0,
    cleaning: false,
    load: function() {
        return Promise.all([ uci.load('overlay_restore'), callList().then(checked),
            callDevices().then(checked).catch(function(error) { return { devices: [], error: error.message }; }) ]);
    },

    saveSettings: function() {
        return this.map.save()
            .then(function() { return callCommit('overlay_restore').then(checked); })
            .then(function() { return ui.changes.init(); });
    },

    inspect: function(ev) {
        return this.saveSettings()
            .then(function() { return ui.uploadFile('/tmp/overlay-restore-upload.tar.gz'); })
            .then(function() { return callPrepare().then(checked); })
            .then(L.bind(function(result) { this.taskId = result.id; return this.refresh(); }, this))
            .catch(function(error) { ui.addNotification(null, E('p', error.message)); });
    },

    inspectRouter: function() {
        const path = this.backupPath.value.trim();
        if (!/^\/.*\.(tar\.gz|tgz)$/.test(path)) {
            ui.addNotification(null, E('p', '请选择或输入路由器上的 .tar.gz / .tgz 备份绝对路径。'));
            return Promise.resolve();
        }
        return this.saveSettings()
            .then(function() { return callPrepare(path).then(checked); })
            .then(L.bind(function(result) { this.taskId = result.id; return this.refresh(); }, this))
            .catch(function(error) { ui.addNotification(null, E('p', error.message)); });
    },

    confirm: function(task) {
        const settings = task.plan.settings;
        ui.showModal('确认恢复计划', [
            E('p', '将覆盖预览中的配置和自定义文件，并在重启后恢复所选软件包。'),
            E('p', settings.clean_overlay ? '将使用当前固件的基础系统与包数据库，提前安装恢复工具及所需 DNS / 代理程序，再在干净环境中恢复配置。' :
                '当前内核、包管理状态、软件源、公钥和恢复工具会保留。'),
            settings.clean_overlay ? E('p', { 'class': 'alert-message warning' },
                '将重建 ' + settings.overlay_target.device + '（' + settings.overlay_target.filesystem + '）上用于系统的 overlay，并停止服务、自动重启。旧 overlay 会保留以便回退；该分区上的其他目录会保留。') : '',
            settings.extroot_uuid ? E('p', { 'class': 'alert-message warning' },
                '将按 UUID ' + settings.extroot_uuid + ' 重新启用此分区为 extroot。当前系统保留，回退时恢复当前系统及原挂载配置。') : '',
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

    confirmOverlayAction: function(task, rollback) {
        ui.showModal(rollback ? '回退到清理前的环境' : '删除保留的另一份 overlay', [
            E('p', rollback ? '将停止服务并自动重启，恢复清理前的完整 overlay。当前恢复后的环境会保留在暂存目录中。' :
                '将永久删除暂存目录中的另一份 overlay，包含其中的软件、配置和自定义文件。删除后无法再通过它回退。'),
            E('p', '暂存目录：' + task.clean_overlay.directory),
            E('p', rollback ? '回退后使用清理前的网络地址和登录凭据。' : '当前正在使用的 overlay 和该分区上其他目录会保留。'),
            E('div', { 'class': 'right' }, [
                E('button', { 'class': 'btn', 'click': ui.hideModal }, '取消'), ' ',
                E('button', { 'class': 'btn cbi-button-negative', 'click': ui.createHandlerFn(this, function() {
                    this.cleaning = true;
                    this.refreshRevision++;
                    return (rollback ? callRollback : callDiscardOverlay)(task.id, task.id).then(checked)
                        .then(L.bind(function() { ui.hideModal(); }, this))
                        .catch(function(error) { ui.addNotification(null, E('p', error.message)); })
                        .finally(L.bind(function() { this.cleaning = false; return this.refresh(); }, this));
                }) }, rollback ? '回退并重启' : '永久删除暂存 overlay')
            ])
        ]);
    },

    showUsage: function() {
        return callUsage().then(checked).then(function(usage) {
            ui.showModal('任务文件占用', [
                E('p', '任务文件：' + formatSize(usage.task_bytes) + '（包含记录、日志、待恢复文件和覆盖前保存的原件）。'),
                E('p', '临时检查文件：' + formatSize(usage.temporary_bytes) + '。'),
                E('p', '任务所在磁盘剩余空间：' + formatSize(usage.free_bytes) + '。'),
                E('p', '这里显示文件大小，磁盘实际占用可能因压缩和文件系统而不同。'),
                E('div', { 'class': 'right' }, [ E('button', { 'class': 'btn', 'click': ui.hideModal }, '关闭') ])
            ]);
        }).catch(function(error) { ui.addNotification(null, E('p', error.message)); });
    },

    confirmCleanup: function(task) {
        const preview = task && task.status == 'ready';
        ui.showModal(task ? (preview ? '取消并删除任务' : '删除任务') : '清理已结束的任务', [
            E('p', task ? '将删除任务 ' + task.id.slice(0, 8) + ' 的记录、日志、暂存文件和覆盖前保存的原件。' :
                '将删除所有已完成和检查失败任务的记录、日志、暂存文件及覆盖前保存的原件。等待确认和未完成的恢复任务会保留。'),
            E('p', '路由器当前配置和磁盘上的原始备份文件会保留。删除的任务资料无法恢复。'),
            preview ? E('p', '以后要使用这个备份恢复，需要重新检查备份。') : '',
            E('div', { 'class': 'right' }, [
                E('button', { 'class': 'btn', 'click': ui.hideModal }, '取消'), ' ',
                E('button', { 'class': 'btn cbi-button-negative', 'click': ui.createHandlerFn(this, function() {
                    this.cleaning = true;
                    ++this.refreshRevision;
                    return (task ? callRemove(task.id, task.id) : callCleanup()).then(checked).then(L.bind(function(result) {
                        ui.hideModal();
                        const removed = result.removed || [];
                        if (removed.indexOf(this.taskId) >= 0)
                            this.taskId = null;
                        this.cleaning = false;
                        return this.refresh().then(function() {
                            ui.addNotification(null, E('p', '已清理 ' + removed.length + ' 个任务。' +
                                (result.skipped_busy ? '正在使用的任务已跳过，可稍后再清理。' : '')), 'info');
                        });
                    }, this)).catch(function(error) {
                        ui.addNotification(null, E('p', error.message));
                    }).finally(L.bind(function() { this.cleaning = false; }, this));
                }) }, task ? '删除此任务' : '清理已结束的任务')
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
            body.push(E('p', settings.extroot_uuid ? '将重新启用 ' + settings.overlay_target.device + ' 为 extroot，UUID：' + settings.extroot_uuid :
                settings.keep_current_extroot ? '保留当前系统的 extroot 挂载配置。' : '使用备份中的完整 fstab。'));
            body.push(E('details', { 'data-section': 'files' }, [
                E('summary', '待恢复文件（最多显示 200 项）'),
                E('pre', { 'style': 'max-height:260px;overflow:auto' }, plan.files.map(function(file) { return file.path; }).join('\n'))
            ]));
            body.push(E('details', { 'data-section': 'packages' }, [
                E('summary', '软件安装及移除计划'),
                E('pre', '当前源安装：\n' + settings.install_packages.join(' ') + '\n\nmyfeed 安装：\n' + settings.myfeed_packages.join(' ') +
                    '\n\n可选安装：\n' + settings.optional_packages.join(' ') + '\n\n移除：\n' + settings.remove_packages.join(' '))
            ]));
            body.push(E('details', { 'data-section': 'skipped' }, [ E('summary', '跳过的备份内容'), E('pre', JSON.stringify(plan.skipped, null, 2)) ]));
        }
        if (task.status == 'ready' && L.hasViewPermission())
            body.push(E('button', { 'class': 'btn cbi-button-action', 'click': L.bind(this.confirm, this, task) }, '确认恢复计划'));
        if ((task.status == 'failed_clean' || task.status == 'failed_prepare' || task.status == 'failed_packages' || task.status == 'complete_with_warnings') && L.hasViewPermission())
            body.push(E('button', { 'class': 'btn', 'click': ui.createHandlerFn(this, function() {
                return callRetry(task.id).then(checked).then(L.bind(this.refresh, this)).catch(function(error) {
                    ui.addNotification(null, E('p', error.message));
                });
            }) }, task.status == 'failed_clean' ? '重试准备干净 overlay' : '重试软件及服务修复'));
        if (task.clean_overlay && task.clean_overlay.retained) {
            body.push(E('p', '另一份 overlay 保留在：' + task.clean_overlay.directory + '。清理任务历史不会删除这份环境。'));
            if (task.clean_overlay.unavailable)
                body.push(E('p', { 'class': 'alert-message warning' }, '暂存环境操作受限：' + task.clean_overlay.unavailable));
            if (task.can_rollback_overlay && L.hasViewPermission())
                body.push(E('button', { 'class': 'btn', 'style': 'margin:4px',
                    'click': L.bind(this.confirmOverlayAction, this, task, true) }, '回退到清理前环境'));
            if (task.can_discard_overlay && L.hasViewPermission())
                body.push(E('button', { 'class': 'btn cbi-button-negative', 'style': 'margin:4px',
                    'click': L.bind(this.confirmOverlayAction, this, task, false) }, '删除暂存 overlay'));
        }
        if (task.can_remove && L.hasViewPermission())
            body.push(E('button', { 'class': 'btn cbi-button-negative', 'style': 'margin:4px',
                'click': L.bind(this.confirmCleanup, this, task) }, task.status == 'ready' ? '取消并删除任务' : '删除任务'));
        const rows = Object.keys(task.packages || {}).map(function(name) {
            const item = task.packages[name];
            return E('tr', { 'class': 'tr' }, [ E('td', { 'class': 'td' }, name), E('td', { 'class': 'td' }, item.operation),
                E('td', { 'class': 'td' }, item.status), E('td', { 'class': 'td' }, item.message || '') ]);
        });
        if (rows.length)
            body.push(E('table', { 'class': 'table' }, [ E('tr', { 'class': 'tr table-titles' },
                [ '软件包', '操作', '结果', '说明' ].map(function(title) { return E('th', { 'class': 'th' }, title); })) ].concat(rows)));
        body.push(E('details', { 'data-section': 'log', 'open': '' }, [ E('summary', '执行日志'),
            E('p', '每个任务日志最多保留 1 MiB 的最新内容，此处显示末尾约 40 KB。删除任务时会一并删除日志。'),
            E('pre', { 'style': 'max-height:320px;overflow:auto;white-space:pre-wrap' }, task.log || '等待执行…') ]));
        return E('div', { 'class': 'cbi-section', 'data-task-id': task.id }, body);
    },

    updateTask: function(task) {
        const snapshot = JSON.stringify(task);
        if (snapshot == this.taskSnapshot)
            return;
        const previous = this.status.firstElementChild;
        const sameTask = previous && previous.getAttribute('data-task-id') == task.id;
        const sections = {};
        let focusedSection = null;
        if (sameTask) {
            previous.querySelectorAll('details[data-section]').forEach(function(section) {
                const key = section.getAttribute('data-section'), pre = section.querySelector('pre');
                sections[key] = { open: section.open, top: pre ? pre.scrollTop : 0, left: pre ? pre.scrollLeft : 0 };
                if (section.querySelector('summary') == document.activeElement)
                    focusedSection = key;
            });
        }
        const panel = this.renderTask(task), scrollX = window.scrollX, scrollY = window.scrollY;
        panel.querySelectorAll('details[data-section]').forEach(function(section) {
            const state = sections[section.getAttribute('data-section')];
            if (state)
                section.open = state.open;
        });
        dom.content(this.status, panel);
        panel.querySelectorAll('details[data-section]').forEach(function(section) {
            const key = section.getAttribute('data-section'), state = sections[key], pre = section.querySelector('pre');
            if (state && pre) {
                pre.scrollTop = state.top;
                pre.scrollLeft = state.left;
            }
            if (key == focusedSection)
                section.querySelector('summary').focus({ preventScroll: true });
        });
        if (sameTask)
            window.scrollTo(scrollX, scrollY);
        this.taskSnapshot = snapshot;
    },

    refresh: function() {
        if (this.cleaning)
            return Promise.resolve();
        const revision = ++this.refreshRevision;
        return callList().then(checked).then(L.bind(function(result) {
            if (revision != this.refreshRevision)
                return null;
            const tasks = result.tasks || [];
            const snapshot = JSON.stringify(tasks.map(function(task) { return [ task.id, task.status ]; }));
            if (snapshot != this.historySnapshot) {
                dom.content(this.history, [ E('h3', '最近的恢复任务'), this.historyInfo,
                    E('div', { 'style': 'display:flex;flex-wrap:wrap;gap:8px;margin-bottom:8px' }, [ this.historyUsage, this.historyCleanup ])
                ].concat(tasks.map(L.bind(function(task) {
                    return E('button', { 'class': 'btn', 'style': 'margin:4px', 'click': L.bind(function() {
                        this.taskId = task.id;
                        return this.refresh();
                    }, this) }, labels[task.status] + ' · ' + task.id.slice(0, 8));
                }, this))));
                this.historySnapshot = snapshot;
            }
            dom.content(this.historyInfo, '显示最近 10 条，共 ' + result.total_tasks + ' / ' + result.max_tasks +
                ' 个任务。达到上限后需清理已结束的任务，或取消不再使用的待确认任务，才能检查新备份。每个任务日志上限 ' + formatSize(result.max_log_bytes) + '。');
            this.historyCleanup.disabled = !L.hasViewPermission() || !result.cleanup_count;
            if (this.taskId && Array.isArray(result.task_ids) && result.task_ids.indexOf(this.taskId) < 0)
                this.taskId = null;
            if (!this.taskId && result.tasks && result.tasks.length)
                this.taskId = result.tasks[0].id;
            return this.taskId ? callStatus(this.taskId).then(checked) : null;
        }, this)).then(L.bind(function(task) {
            if (revision != this.refreshRevision)
                return;
            this.upload.disabled = uploadDisabled(task);
            this.inspectLocal.disabled = this.upload.disabled;
            this.backupPath.disabled = this.upload.disabled;
            this.quickfile.disabled = this.upload.disabled;
            if (!task) {
                dom.content(this.status, []);
                this.taskSnapshot = null;
                return;
            }
            this.updateTask(task);
            if ([ 'awaiting_reboot', 'awaiting_clean_boot', 'awaiting_rollback_boot' ].indexOf(task.status) >= 0 &&
                task.plan.settings.reboot && !task.reboot_failed && !this.reconnecting) {
                this.reconnecting = true;
                ui.showModal('正在等待重启', [ E('p', { 'class': 'spinning' }, '重启后重新登录此页面，可查看软件包恢复结果。') ]);
                const addresses = [ window.location.host ];
                if (task.status == 'awaiting_rollback_boot' && task.clean_overlay.original_lan)
                    addresses.push(task.clean_overlay.original_lan);
                else if (task.plan.lan_ip)
                    addresses.push(task.plan.lan_ip);
                ui.awaitReconnect.apply(ui, addresses);
            }
        }, this));
    },

    render: function(data) {
        const map = this.map = new form.Map('overlay_restore', '备份迁移恢复',
            '上传备份或直接选择路由器上的 overlay / sysupgrade 备份，先检查恢复计划，再迁移配置和自定义文件。软件包会在重启后从当前软件源重新安装。修改恢复选项后请重新检查备份，已有计划使用检查时的设置。');
        map.readonly = !L.hasViewPermission();
        const section = map.section(form.NamedSection, 'main', 'restore');
        section.tab('general', '恢复选项');
        section.tab('packages', '软件包');
        section.tab('services', '服务修复');
        section.tab('limits', '备份限制');
        [
            [ 'clean_overlay', '恢复前重建干净 overlay', '勾选：自动准备当前固件的干净环境，停止服务后切换并重启恢复。可使用当前 overlay，也可选择升级后需要重新启用的外部分区。旧环境保留供回退，其他磁盘目录保留。需要保持「保留当前 extroot」和「自动重启」开启，并能访问软件源。默认关闭。' ],
            [ 'keep_current_extroot', '保留当前 extroot', '勾选（默认）：保留当前用于系统的 extroot 挂载，恢复备份中的其他挂载项。不勾选：使用备份中的完整挂载配置。重建干净 overlay 时必须开启，挂载目标以「恢复目标 overlay」的选择为准。' ],
            [ 'keep_network', '保留当前网络、防火墙和 DHCP 配置', '勾选：保留当前三项配置。不勾选（默认）：恢复备份中的对应配置，LAN 地址可能改变。Wi-Fi、SmartDNS 和 Nikki 配置不受此选项保护。' ],
            [ 'restore_credentials', '恢复备份中的账号及 SSH 凭据', '勾选（默认）：恢复备份中的账号、密码和 SSH 凭据，重连时可能需要使用备份密码。不勾选：保留当前登录凭据。' ],
            [ 'reboot', '迁移完成后自动重启', '勾选（默认）：配置迁移后自动重启，再安装软件并修复服务。不勾选：等待手动重启后继续。' ]
        ].forEach(function(item) {
            const option = section.taboption('general', form.Flag, item[0], item[1], item[2]);
            option.rmempty = false;
        });
        const target = section.taboption('general', form.ListValue, 'overlay_device', '恢复目标 overlay',
            '选择当前 overlay，或需要重新启用为 extroot 的 ext4 / f2fs 分区。执行时会临时挂载未挂载分区，重建系统 upper/work，并按 UUID 启用 extroot；不格式化分区，其他目录保留。插入磁盘后刷新页面可重新读取分区。');
        target.depends('clean_overlay', '1');
        target.rmempty = false;
        target.value('', data[2].current ? '当前 overlay（' + data[2].current.device + '）' : '当前 overlay');
        const selectedDevice = uci.get('overlay_restore', 'main', 'overlay_device');
        if (selectedDevice && data[2].current && selectedDevice == data[2].current.device)
            target.value(selectedDevice, '当前 extroot（' + selectedDevice + '）');
        (data[2].devices || []).forEach(function(device) {
            target.value(device.device, device.device + ' · ' + device.filesystem +
                (device.label ? ' · ' + device.label : '') + ' · UUID ' + device.uuid +
                (device.mountpoint ? ' · ' + device.mountpoint : ' · 未挂载'));
        });
        if (selectedDevice && !target.keylist.includes(selectedDevice))
            target.value(selectedDevice, selectedDevice + ' · 当前不可用，请重新选择');
        if (data[2].error)
            target.description += ' 分区读取失败：' + data[2].error;
        [ [ 'install_packages', '从当前源安装' ], [ 'myfeed_packages', '从 myfeed 安装' ], [ 'optional_packages', '从 myfeed 尝试安装' ],
          [ 'remove_packages', '移除预装 LuCI 软件包' ] ].forEach(function(item) {
            section.taboption('packages', form.DynamicList, item[0], item[1]);
        });
        section.taboption('packages', form.Value, 'myfeed_repo', '默认 myfeed 地址', '系统已有 00-myfeed.list 时优先使用其中的地址。');
        section.taboption('packages', form.Value, 'myfeed_key_url', 'myfeed 公钥地址');
        section.taboption('services', form.Flag, 'iptv_enable', '恢复 IPTV Refresh').rmempty = false;
        [ [ 'iptv_repo_root', 'IPTV 数据目录' ], [ 'ha_config_root', 'Home Assistant 配置目录' ] ].forEach(function(item) {
            section.taboption('services', DirectoryPath, item[0], item[1], '通过 QuickFile 选择目录，也可直接填写绝对路径。');
        });
        [ [ 'iptv_refresh_iface', 'IPTV 网络接口' ], [ 'iptv_refresh_host', '刷新服务监听地址' ],
          [ 'iptv_refresh_port', '刷新服务端口' ], [ 'iptv_public_url', 'IPTV 对外地址' ] ].forEach(function(item) {
            section.taboption('services', form.Value, item[0], item[1]);
        });
        section.taboption('services', form.DynamicList, 'iptv_refresh_allow_ips', '刷新服务允许的 IP');
        section.taboption('services', form.DynamicList, 'iptv_nginx_allow_ips', '代理允许的 IP / 网段');
        const token = section.taboption('services', form.Value, 'iptv_refresh_token', '刷新服务令牌', '留空时生成随机令牌。');
        token.password = true;
        [ [ 'max_upload_mb', '备份文件大小上限（MiB）', 'range(1,1024)' ], [ 'max_expanded_mb', '展开大小上限（MiB）', 'range(8,4096)' ] ].forEach(function(item) {
            section.taboption('limits', form.Value, item[0], item[1]).datatype = item[2];
        });
        return map.render().then(L.bind(function(formNode) {
            const task = (data[1].tasks || [])[0];
            this.status = E('div');
            this.history = E('div', { 'class': 'cbi-section' });
            this.historyInfo = E('p', { 'class': 'cbi-section-descr' });
            this.historyUsage = E('button', { 'class': 'btn', 'click': ui.createHandlerFn(this, 'showUsage') }, '查看占用');
            this.historyCleanup = E('button', { 'class': 'btn cbi-button-negative', 'disabled': !L.hasViewPermission() || !data[1].cleanup_count,
                'click': L.bind(this.confirmCleanup, this, null) }, '清理已结束的任务');
            this.upload = E('button', { 'class': 'btn cbi-button-action', 'disabled': uploadDisabled(task) || null,
                'click': ui.createHandlerFn(this, 'inspect') }, '上传并检查备份');
            this.backupPath = E('input', { 'type': 'text', 'class': 'cbi-input-text', 'aria-label': '路由器备份路径',
                'placeholder': '/mnt/backup/overlay_backup.tar.gz', 'disabled': uploadDisabled(task) || null,
                'style': 'width:100%;max-width:640px;box-sizing:border-box' });
            this.inspectLocal = E('button', { 'class': 'btn cbi-button-action', 'disabled': uploadDisabled(task) || null,
                'click': ui.createHandlerFn(this, 'inspectRouter') }, '检查路由器上的备份');
            this.quickfile = E('button', {
                'class': 'btn', 'disabled': uploadDisabled(task) || null,
                'click': ui.createHandlerFn(this, function() {
                    return openQuickFile({
                        directory: false, value: this.backupPath.value,
                        enabled: L.bind(function() { return !this.upload.disabled; }, this),
                        select: L.bind(function(path) { this.backupPath.value = path; }, this)
                    });
                })
            }, '打开 QuickFile 选择备份');
            this.taskId = task ? task.id : null;
            poll.add(L.bind(function() { return this.refresh().catch(function() {}); }, this), 3);
            return E('div', {}, [ formNode, E('div', { 'class': 'cbi-section' }, [
                E('h3', '选择备份'), this.upload,
                E('p', '通过 QuickFile 选择路由器上的备份，也可直接输入绝对路径。检查后原文件会保留。'),
                this.backupPath,
                E('div', { 'style': 'display:flex;flex-wrap:wrap;gap:8px;margin-top:8px' }, [ this.quickfile, this.inspectLocal ])
            ]), this.status, this.history ]);
        }, this));
    },

    handleSaveApply: function() {
        return this.saveSettings()
            .then(function() { ui.addNotification(null, E('p', '恢复设置已保存并应用，后续检查将使用这些设置。'), 'info'); })
            .catch(function(error) { ui.addNotification(null, E('p', error.message)); });
    },
    handleSave: function() {
        return this.saveSettings()
            .then(function() { ui.addNotification(null, E('p', '恢复设置已保存，后续检查将使用这些设置。'), 'info'); })
            .catch(function(error) { ui.addNotification(null, E('p', error.message)); });
    }
});
