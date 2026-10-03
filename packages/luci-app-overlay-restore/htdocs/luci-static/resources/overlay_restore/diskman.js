'use strict';
'require baseclass';
'require ui';
'require dom';

function choices(data) {
    const result = new Map();
    (data.devices || []).forEach(function(device) {
        result.set(device.device, Object.assign({ value: device.device }, device));
    });
    if (data.current && data.current.device)
        result.set(data.current.device, Object.assign({ value: '', current: true }, data.current));
    return result;
}

function deviceName(text) {
    const name = String(text || '').trim().split(/\s+/)[0].replace(/^\/dev\//, '');
    return /^[a-zA-Z0-9-]{1,64}$/.test(name) ? '/dev/' + name : null;
}

return baseclass.extend({
    open: function(options) {
        if (!options.enabled())
            return Promise.resolve();
        const page = new URL(L.url('admin/system/diskman'), window.location.origin);
        const controller = new AbortController();
        const panel = E('div', {}, E('p', { 'class': 'spinning' }, '正在打开 DiskMan…'));
        const notice = E('p', { 'class': 'alert-message warning', 'style': 'display:none' });
        let frame = null, closed = false, selecting = false, available = new Map();
        let detach = function() {};
        const showError = function(message) {
            if (!closed) {
                dom.content(notice, message);
                notice.style.display = '';
            }
        };
        const close = function() {
            closed = true;
            controller.abort();
            detach();
            if (frame)
                frame.remove();
            ui.hideModal();
        };
        const navigate = function(url) {
            detach();
            frame.style.visibility = 'hidden';
            frame.src = url.href;
        };
        const choose = function(device) {
            if (closed || selecting || !options.enabled())
                return;
            const selected = available.get(device);
            if (!selected || !options.values.includes(selected.value)) {
                showError('此分区当前不能作为恢复目标。请选择可用的 ext4 / f2fs 分区；插入新磁盘后请刷新恢复页面。');
                return;
            }
            selecting = true;
            notice.style.display = 'none';
            options.load().then(function(data) {
                const current = choices(data).get(device);
                if (!current || current.value !== selected.value ||
                    current.filesystem !== selected.filesystem || current.uuid !== selected.uuid)
                    throw new Error('分区状态已变化，请刷新页面后重新选择。');
                if (!closed && options.enabled()) {
                    options.select(current.value, data);
                    close();
                }
            }).catch(function(error) { showError(error.message); })
                .finally(function() { selecting = false; });
        };
        const attach = function() {
            detach();
            if (closed)
                return;
            try {
                const location = new URL(frame.contentWindow.location.href);
                if (location.origin !== page.origin || location.pathname !== page.pathname &&
                    !location.pathname.startsWith(page.pathname + '/'))
                    throw new Error('DiskMan 页面不可用，请确认「系统 → 磁盘管理」能正常打开。');
                const document = frame.contentDocument;
                if (document.querySelector('input[name="luci_username"]'))
                    throw new Error('DiskMan 登录会话已失效，请刷新页面重新登录。');
                // This embedded page is a selector. Disable native actions before
                // exposing it; navigation and rescan are handled as read-only GETs.
                const style = document.createElement('style');
                style.textContent = 'header,footer,nav,.cbi-page-actions,.cbi-tabmenu,button,input,select,textarea,a{display:none!important}' +
                    '[data-overlay-browse]{display:inline-block!important}' +
                    '[data-overlay-device]{cursor:pointer;outline-offset:-3px}' +
                    '[data-overlay-device][aria-disabled="true"]{cursor:not-allowed;opacity:.55}' +
                    '[data-overlay-device][aria-disabled="false"]:hover,[data-overlay-device]:focus{outline:3px solid #409eff}' +
                    '.dm-part-segment[aria-disabled="false"]{text-decoration:underline}' +
                    '.cbi-map{margin:0!important}';
                document.head.appendChild(style);
                const decorate = function() {
                    document.querySelectorAll('button,input,select,textarea').forEach(function(control) {
                        control.disabled = true;
                    });
                    const mark = function(element, name) {
                        if (!name)
                            return;
                        const selected = available.get(name);
                        const enabled = !!selected && options.values.includes(selected.value);
                        element.setAttribute('data-overlay-device', name);
                        element.setAttribute('role', 'button');
                        element.setAttribute('aria-disabled', String(!enabled));
                        element.setAttribute('tabindex', enabled ? '0' : '-1');
                        if (enabled)
                            element.title = '点击选择 ' + name + ' · ' + selected.filesystem +
                                (selected.uuid ? ' · UUID ' + selected.uuid : ' · 当前 overlay');
                    };
                    document.querySelectorAll('.dm-part-segment:not(.dm-part-free)').forEach(function(segment) {
                        mark(segment, deviceName(segment.textContent));
                    });
                    document.querySelectorAll('.cbi-section-table tr,.cbi-section-table .tr').forEach(function(row) {
                        if (row.children.length < 10 || row.classList.contains('table-titles'))
                            return;
                        mark(row, deviceName(row.children[0].textContent));
                        const fs = row.children[9];
                        const button = fs && fs.querySelector('button,input[type="button"]');
                        if (button && !fs.querySelector('[data-overlay-filesystem]'))
                            fs.appendChild(E('span', { 'data-overlay-filesystem': '' }, button.value || button.textContent));
                    });
                    document.querySelectorAll('.dkm-card').forEach(function(card) {
                        if (!card.querySelector('.dm-part-bar'))
                            return;
                        const title = card.querySelector('.dkm-card-title');
                        const device = title && title.textContent.match(/\(\/dev\/([a-zA-Z0-9-]{1,64})\)/);
                        const button = card.querySelector('.dkm-card-actions .cbi-button-action');
                        if (device && button) {
                            button.setAttribute('data-overlay-browse', device[1]);
                            if (button.textContent !== '查看分区')
                                button.textContent = '查看分区';
                            button.disabled = false;
                        }
                    });
                    frame.style.visibility = '';
                };
                const click = function(ev) {
                    const target = ev.target.closest ? ev.target : ev.target.parentElement;
                    if (!target)
                        return;
                    const browse = target.closest('[data-overlay-browse]');
                    const entry = target.closest('[data-overlay-device]');
                    // Intercept before DiskMan's formatting, mount and delete
                    // handlers, including controls inside selectable table rows.
                    ev.preventDefault();
                    ev.stopImmediatePropagation();
                    if (browse) {
                        const url = new URL(L.url('admin/system/diskman/partition', browse.getAttribute('data-overlay-browse')), page.origin);
                        navigate(url);
                    }
                    else if (entry)
                        choose(entry.getAttribute('data-overlay-device'));
                };
                const keydown = function(ev) {
                    if (ev.key === 'Enter' || ev.key === ' ') {
                        const target = ev.target.closest && ev.target.closest('[data-overlay-device],[data-overlay-browse]');
                        if (target)
                            click(ev);
                    }
                };
                const submit = function(ev) { ev.preventDefault(); ev.stopImmediatePropagation(); };
                document.addEventListener('click', click, true);
                document.addEventListener('keydown', keydown, true);
                document.addEventListener('submit', submit, true);
                const observer = new frame.contentWindow.MutationObserver(decorate);
                observer.observe(document.body, { childList: true, subtree: true });
                detach = function() {
                    observer.disconnect();
                    document.removeEventListener('click', click, true);
                    document.removeEventListener('keydown', keydown, true);
                    document.removeEventListener('submit', submit, true);
                };
                decorate();
            }
            catch (error) {
                dom.content(panel, E('p', { 'class': 'alert-message warning' }, error.message));
            }
        };
        ui.showModal('DiskMan · 选择恢复分区', [
            E('style', '.modal.overlay-restore-diskman{width:calc(100vw - 32px);max-width:1280px;box-sizing:border-box}'),
            E('p', '点击磁盘分区条或分区列表中的目标分区，即可回填恢复目标。也可点击「查看分区」核对详细信息。选择后仍需检查备份并确认恢复。'),
            panel, notice,
            E('div', { 'class': 'right' }, [
                E('button', { 'class': 'btn', 'click': close }, '关闭'), ' ',
                E('button', { 'class': 'btn', 'click': function() { if (frame) navigate(page); } }, '扫描磁盘 / 返回磁盘列表')
            ])
        ], 'overlay-restore-diskman');
        const timeout = window.setTimeout(function() { controller.abort(); }, 8000);
        return options.load().then(function(data) {
            available = choices(data);
            return fetch(page.href, { credentials: 'same-origin', signal: controller.signal });
        }).then(function(response) {
            if (!response.ok || new URL(response.url).origin !== page.origin)
                throw new Error('DiskMan 页面不可用');
            return response.text();
        }).then(function(html) {
            if (!/diskman\/overview|diskman\/disks|Disk\s?Man/i.test(html))
                throw new Error('DiskMan 页面不可用');
            if (closed || !panel.isConnected)
                return;
            frame = E('iframe', {
                'src': page.href, 'title': 'DiskMan 分区选择',
                'style': 'display:block;width:100%;height:60vh;min-height:260px;border:0;border-radius:6px;visibility:hidden'
            });
            frame.addEventListener('load', attach);
            dom.content(panel, frame);
        }).catch(function() {
            if (!closed)
                dom.content(panel, E('p', { 'class': 'alert-message warning' },
                    'DiskMan 暂不可用。请确认已安装 luci-app-diskman，并且「系统 → 磁盘管理」能正常打开。也可关闭弹窗，使用恢复目标下拉框选择分区。'));
        }).finally(function() { window.clearTimeout(timeout); });
    }
});
