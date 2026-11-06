'use strict';
'require baseclass';
'require rpc';
'require ui';

const callStatus = rpc.declare({ object: 'luci.fiend', method: 'status' });
const callProfiles = rpc.declare({ object: 'luci.fiend', method: 'profiles' });
const callSettings = rpc.declare({ object: 'luci.fiend', method: 'settings' });
const callLogs = rpc.declare({ object: 'luci.fiend', method: 'logs', params: [ 'limit' ] });
const callRequest = rpc.declare({ object: 'luci.fiend', method: 'request', params: [ 'request' ] });

const KINDS = [
	[ 'proxy', _('Tunnel'), _('Send through the Fiend server') ],
	[ 'dpi', _('DPI bypass'), _('Connect directly and split the TLS greeting') ],
	[ 'direct', _('Direct'), _('Connect directly') ],
	[ 'block', _('Block'), _('Refuse the connection') ]
];

const STYLE = `
.fiend-card{border:1px solid var(--border-color-medium,#ddd);border-radius:8px;padding:16px 20px;margin-bottom:16px;background:var(--background-color-high,transparent)}
.fiend-row{display:flex;align-items:center;gap:12px;flex-wrap:wrap}
.fiend-grow{flex:1;min-width:200px}
.fiend-state{font-size:1.4em;font-weight:600;display:flex;align-items:center;gap:10px}
.fiend-dot{width:12px;height:12px;border-radius:50%;background:var(--border-color-high,#999);flex:none}
.fiend-dot.on{background:var(--success-color-medium,#2a2)}
.fiend-dot.busy{background:var(--warn-color-medium,#e90);animation:fiend-pulse 1s infinite alternate}
.fiend-dot.bad{background:var(--error-color-medium,#c33)}
@keyframes fiend-pulse{from{opacity:1}to{opacity:.3}}
.fiend-muted{opacity:.7}
.fiend-error{color:var(--error-color-medium,#c33)}
.fiend-tiles{display:grid;grid-template-columns:repeat(auto-fit,minmax(130px,1fr));gap:12px;margin-bottom:16px}
.fiend-tile{border:1px solid var(--border-color-medium,#ddd);border-radius:8px;padding:12px 16px}
.fiend-tile-label{font-size:.85em;opacity:.7}
.fiend-tile-value{font-size:1.35em;font-weight:600;margin-top:4px;font-variant-numeric:tabular-nums;white-space:nowrap}
.fiend-list{border:1px solid var(--border-color-medium,#ddd);border-radius:8px;overflow:hidden;margin-bottom:16px}
.fiend-item{display:flex;align-items:center;gap:14px;padding:12px 16px;border-top:1px solid var(--border-color-medium,#ddd)}
.fiend-item:first-child{border-top:0}
.fiend-item .title{font-weight:600;word-break:break-all}
.fiend-item .meta{font-size:.9em;opacity:.75;margin-top:2px}
.fiend-badge{display:inline-block;border-radius:4px;padding:1px 6px;font-size:.8em;font-weight:600;border:1px solid currentColor;margin-right:6px}
.fiend-empty{padding:24px;text-align:center;opacity:.7}
.fiend-choice{display:grid;grid-template-columns:repeat(auto-fit,minmax(240px,1fr));gap:12px}
.fiend-choice label{display:block;border:1px solid var(--border-color-medium,#ddd);border-radius:8px;padding:12px 16px;cursor:pointer}
.fiend-choice label.selected{border-color:var(--primary-color-medium,#36c);box-shadow:0 0 0 1px var(--primary-color-medium,#36c)}
.fiend-choice strong{margin-left:8px}
.fiend-log{font-family:monospace;font-size:12px;white-space:pre-wrap;word-break:break-all;max-height:70vh;overflow:auto;border:1px solid var(--border-color-medium,#ddd);border-radius:8px;padding:8px 12px}
.fiend-log .warn{color:var(--warn-color-high,#b70)}
.fiend-log .error{color:var(--error-color-medium,#c33)}
.fiend-log .debug,.fiend-log .trace{opacity:.6}
.fiend-rules textarea{width:100%;min-height:180px;font-family:monospace}
`;

function unwrap(result) {
	if (result == null || typeof result != 'object')
		throw new Error(_('Fiend gave no answer'));
	if (result.error)
		throw new Error(result.error.message || result.error.code);
	return result.reply;
}

function plural(count, one, many) {
	return count == 1 ? one.format(count) : many.format(count);
}

return baseclass.extend({
	KINDS: KINDS,

	status() { return callStatus().then(unwrap); },
	profiles() { return callProfiles().then(unwrap).then((reply) => reply.profiles); },
	settings() { return callSettings().then(unwrap); },
	logs(limit) { return callLogs(limit).then(unwrap).then((reply) => reply.entries); },

	request(type, fields) {
		return callRequest(Object.assign({ type: type }, fields || {})).then(unwrap);
	},

	style() {
		if (!document.getElementById('fiend-style'))
			document.head.appendChild(E('style', { id: 'fiend-style' }, STYLE));
	},

	fail(error) {
		ui.addNotification(null, E('p', error.message || String(error)), 'danger');
	},

	kindName(kind) {
		const found = KINDS.find((item) => item[0] == kind);
		return found ? found[1] : kind;
	},

	settingsOf(reply) {
		const settings = Object.assign({}, reply);
		delete settings.type;
		return settings;
	},

	bytes(value) {
		const units = [ 'B', 'KiB', 'MiB', 'GiB', 'TiB' ];
		let size = value || 0, unit = 0;
		while (size >= 1024 && unit < units.length - 1) {
			size /= 1024;
			unit++;
		}
		return unit ? '%.1f %s'.format(size, units[unit]) : '%d B'.format(size);
	},

	rate(value) {
		return this.bytes(value) + '/s';
	},

	duration(ms) {
		const seconds = Math.max(0, Math.floor(ms / 1000));
		const days = Math.floor(seconds / 86400);
		const hours = Math.floor(seconds % 86400 / 3600);
		const minutes = Math.floor(seconds % 3600 / 60);
		if (days)
			return _('%dd %dh').format(days, hours);
		if (hours)
			return _('%dh %dm').format(hours, minutes);
		if (minutes)
			return _('%dm').format(minutes);
		return _('%ds').format(seconds);
	},

	ago(ms) {
		if (!ms)
			return _('not loaded');
		const minutes = Math.floor((Date.now() - ms) / 60000);
		if (minutes < 1)
			return _('now');
		if (minutes < 60)
			return plural(minutes, _('%d minute ago'), _('%d minutes ago'));
		const hours = Math.floor(minutes / 60);
		if (hours < 24)
			return plural(hours, _('%d hour ago'), _('%d hours ago'));
		const days = Math.floor(hours / 24);
		return plural(days, _('%d day ago'), _('%d days ago'));
	},

	rules(count) {
		return plural(count, _('%d rule'), _('%d rules'));
	},

	state(status) {
		const state = status && status.state ? status.state : { state: 'unavailable' };
		switch (state.state) {
		case 'connected':
			return { dot: 'on', text: _('Connected') };
		case 'connecting':
			return {
				dot: 'busy',
				text: {
					server: _('Connecting to the server'),
					routes: _('Adding the firewall rules'),
					dns: _('Changing DNS')
				}[state.step] || _('Connecting')
			};
		case 'disconnecting':
			return { dot: 'busy', text: _('Disconnecting') };
		case 'failed':
			return { dot: 'bad', text: _('Not connected'), reason: state.reason };
		case 'unavailable':
			return { dot: 'bad', text: _('Fiend is not running') };
		default:
			return { dot: '', text: _('Disconnected') };
		}
	},

	addProfile(link) {
		return this.request('import_link', { link: link.trim() }).then((profile) =>
			Promise.all([ this.settings(), this.status() ]).then(([ settings, status ]) => {
				const idle = [ 'disconnected', 'failed' ].includes(status.state.state);
				if (settings.connect_at_startup && idle)
					return this.request('connect', { profile_id: profile.id });
			}));
	},

	busy(button, promise) {
		button.disabled = true;
		button.classList.add('spinning');
		return Promise.resolve(promise).finally(() => {
			button.disabled = false;
			button.classList.remove('spinning');
		});
	}
});
