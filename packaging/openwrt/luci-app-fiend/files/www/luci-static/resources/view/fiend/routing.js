'use strict';
'require view';
'require dom';
'require ui';
'require fiend.api as api';

const RULE_FIELDS = { proxy: 'proxy_rules', dpi: 'dpi_rules', direct: 'direct_rules', block: 'block_rules' };

function urlTitle(url) {
	let path = url.split(/[?#]/)[0];
	path = path.includes('://') ? path.split('://')[1] : path;
	const file = path.replace(/\/+$/, '').split('/').pop() || path;
	const stem = file.replace(/\.(srs|txt|list|json)$/, '');
	return stem || url;
}

function kindSelect(value) {
	return E('select', { class: 'cbi-input-select' }, api.KINDS.map((kind) =>
		E('option', { value: kind[0], selected: kind[0] == value ? '' : null }, kind[1])));
}

return view.extend({
	load() {
		return Promise.all([ api.settings(), api.status().catch(() => null) ]);
	},

	reload() {
		return this.load().then((data) => dom.content(this.root, this.body(data)));
	},

	save(change) {
		return api.settings().then((reply) => {
			const settings = api.settingsOf(reply);
			change(settings);
			return api.request('set_settings', { settings: settings });
		});
	},

	reconnectNotice(status) {
		if (!status || !status.restart_required)
			return '';
		return E('div', { class: 'alert-message warning' }, [
			_('The changes apply after you reconnect.'), ' ',
			E('button', {
				class: 'btn cbi-button cbi-button-action',
				click: (ev) => api.busy(ev.target, api.request('connect', { profile_id: status.profile_id }))
					.then(() => this.reload())
					.catch((error) => api.fail(error))
			}, _('Reconnect'))
		]);
	},

	unlisted(settings) {
		const options = [
			[ 'direct', _('Direct'), _('Only the traffic in your Tunnel, DPI bypass and Block lists goes through Fiend. All other traffic stays in the kernel at full speed. Use this on a router.') ],
			[ 'tunnel', _('Tunnel'), _('All traffic goes through the tunnel, except the traffic in your Direct lists.') ]
		];
		return E('div', { class: 'fiend-choice' }, options.map((option) => {
			const selected = settings.default_route == option[0];
			return E('label', { class: selected ? 'selected' : '' }, [
				E('input', {
					type: 'radio',
					name: 'default_route',
					checked: selected ? '' : null,
					change: () => this.save((value) => value.default_route = option[0])
						.then(() => this.reload())
						.catch((error) => api.fail(error))
				}),
				E('strong', {}, option[1]),
				E('div', { class: 'fiend-muted', style: 'margin-top:4px' }, option[2])
			]);
		}));
	},

	editList(item) {
		const name = E('input', { class: 'cbi-input-text', style: 'width:100%', value: item ? item.name : '', placeholder: _('From the address') });
		const url = E('input', { class: 'cbi-input-text', style: 'width:100%', type: 'url', placeholder: 'https://example.com/list.srs' });
		const kind = kindSelect(item ? item.kind : 'proxy');
		const fields = [];
		if (!item)
			fields.push(E('div', { class: 'cbi-value' }, [ E('label', { class: 'cbi-value-title' }, _('Address')), E('div', { class: 'cbi-value-field' }, [
				url, E('div', { class: 'cbi-value-description' }, _('A sing-box rule set (.srs) or a text list with one rule on each line.'))
			]) ]));
		fields.push(E('div', { class: 'cbi-value' }, [ E('label', { class: 'cbi-value-title' }, _('Name')), E('div', { class: 'cbi-value-field' }, name) ]));
		const hint = E('div', { class: 'cbi-value-description' });
		const explain = () => hint.textContent = api.KINDS.find((item) => item[0] == kind.value)[2] + '.';
		kind.addEventListener('change', explain);
		explain();
		fields.push(E('div', { class: 'cbi-value' }, [ E('label', { class: 'cbi-value-title' }, _('Action')), E('div', { class: 'cbi-value-field' }, [ kind, hint ]) ]));
		const save = E('button', {
			class: 'btn cbi-button cbi-button-positive',
			click: () => {
				const request = item
					? api.request('update_subscription', { id: item.id, name: name.value, kind: kind.value })
					: api.request('add_subscription', { url: url.value.trim(), name: name.value, kind: kind.value });
				api.busy(save, request)
					.then((reply) => {
						ui.hideModal();
						if (reply && reply.error)
							api.fail(new Error(_('The list is saved, but it has an error: %s').format(reply.error)));
						return this.reload();
					})
					.catch((error) => api.fail(error));
			}
		}, item ? _('Save') : _('Add list'));
		ui.showModal(item ? _('Edit list') : _('Add a list'), [
			E('div', { class: 'cbi-section' }, fields),
			E('div', { class: 'right' }, [ E('button', { class: 'btn cbi-button', click: ui.hideModal }, _('Cancel')), ' ', save ])
		]);
	},

	removeList(item, title) {
		ui.showModal(_('Remove list'), [
			E('p', {}, _('Remove the list "%s"?').format(title)),
			E('div', { class: 'right' }, [
				E('button', { class: 'btn cbi-button', click: ui.hideModal }, _('Cancel')), ' ',
				E('button', {
					class: 'btn cbi-button cbi-button-negative',
					click: (ev) => api.busy(ev.target, api.request('remove_subscription', { id: item.id }))
						.then(() => { ui.hideModal(); return this.reload(); })
						.catch((error) => { ui.hideModal(); api.fail(error); })
				}, _('Remove'))
			])
		]);
	},

	lists(settings) {
		const items = settings.subscriptions.map((item) => {
			const title = item.name.trim() || urlTitle(item.url);
			const meta = [ E('span', { class: 'fiend-badge' }, api.kindName(item.kind)), api.rules(item.entries), ' · ', api.ago(item.updated_at_ms) ];
			return E('div', { class: 'fiend-item' }, [
				E('div', { class: 'fiend-grow' }, [
					E('div', { class: 'title', title: item.url }, title),
					E('div', { class: 'meta' }, meta),
					item.error ? E('div', { class: 'meta fiend-error' }, item.error) : ''
				]),
				E('button', { class: 'btn cbi-button', click: () => this.editList(item) }, _('Edit')),
				E('button', { class: 'btn cbi-button cbi-button-negative', click: () => this.removeList(item, title) }, _('Remove'))
			]);
		});
		return E('div', { class: 'fiend-list' }, items.length ? items
			: E('div', { class: 'fiend-empty' }, _('Add a list of sites to send them through the tunnel or through DPI bypass.')));
	},

	rules(settings) {
		const panes = api.KINDS.map((kind) => {
			const field = RULE_FIELDS[kind[0]];
			const text = E('textarea', { class: 'cbi-input-textarea', spellcheck: 'false', placeholder: kind[0] == 'block'
				? 'domain:example.com\nfull:www.example.com\nkeyword:example'
				: 'domain:example.com\nfull:www.example.com\nkeyword:example\nip:192.0.2.0/24' }, settings[field] || '');
			const result = E('span', { class: 'fiend-muted' });
			const save = E('button', {
				class: 'btn cbi-button cbi-button-positive',
				click: () => api.busy(save, api.request('set_rule_text', { kind: kind[0], text: text.value }))
					.then((stats) => {
						result.textContent = _('Saved: %d domains, %d exact names, %d keywords, %d networks.')
							.format(stats.domains, stats.exact, stats.keywords, stats.cidrs);
						return api.status().then((status) => dom.content(this.notice, this.reconnectNotice(status)));
					})
					.catch((error) => api.fail(error))
			}, _('Save'));
			return E('div', { 'data-tab': kind[0], 'data-tab-title': kind[1] }, [
				E('p', { class: 'fiend-muted' }, kind[2] + '.'),
				text,
				E('div', { class: 'fiend-row', style: 'margin-top:8px' }, [ save, result ])
			]);
		});
		return E('div', { class: 'fiend-rules' }, panes);
	},

	body(data) {
		const [ settings, status ] = data;
		const refresh = E('button', {
			class: 'btn cbi-button',
			click: () => api.busy(refresh, api.request('refresh_subscriptions'))
				.then(() => this.reload())
				.catch((error) => api.fail(error))
		}, _('Refresh all'));
		this.notice = E('div', {}, this.reconnectNotice(status));
		const rules = this.rules(settings);
		const body = [
			this.notice,
			E('h3', {}, _('Traffic that is not in a list')),
			this.unlisted(settings),
			E('div', { class: 'fiend-row', style: 'margin-top:24px' }, [
				E('h3', { class: 'fiend-grow', style: 'margin:0' }, _('Lists')),
				refresh,
				E('button', { class: 'btn cbi-button cbi-button-add', click: () => this.editList(null) }, _('Add list'))
			]),
			E('p', { class: 'fiend-muted' }, _('Fiend downloads the lists again each day.')),
			this.lists(settings),
			E('h3', {}, _('Your rules')),
			E('p', { class: 'fiend-muted' }, _('One rule on each line: domain:example.com also matches subdomains, full: matches only the name, keyword: matches a part of the name, ip: matches an address or a network.')),
			rules
		];
		requestAnimationFrame(() => ui.tabs.initTabGroup(rules.childNodes));
		return body;
	},

	render(data) {
		api.style();
		this.root = E('div', {}, this.body(data));
		return E('div', {}, [ E('h2', {}, _('Routing')), this.root ]);
	},

	handleSave: null,
	handleSaveApply: null,
	handleReset: null
});
