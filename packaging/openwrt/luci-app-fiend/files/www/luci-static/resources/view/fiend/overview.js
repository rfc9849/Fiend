'use strict';
'require view';
'require poll';
'require dom';
'require ui';
'require fiend.api as api';

return view.extend({
	load() {
		return Promise.all([
			api.status().catch(() => null),
			api.profiles().catch(() => []),
			api.settings().catch(() => null)
		]);
	},

	tile(label) {
		const value = E('div', { class: 'fiend-tile-value' }, '–');
		return [ E('div', { class: 'fiend-tile' }, [ E('div', { class: 'fiend-tile-label' }, label), value ]), value ];
	},

	importCard() {
		const input = E('textarea', {
			class: 'cbi-input-textarea',
			rows: 3,
			style: 'width:100%',
			placeholder: 'fiend://…'
		});
		const button = E('button', {
			class: 'btn cbi-button cbi-button-positive',
			click: () => api.busy(button, api.addProfile(input.value)
				.then(() => window.location.reload())
				.catch((error) => api.fail(error)))
		}, _('Add profile'));
		return E('div', { class: 'fiend-card' }, [
			E('h3', {}, _('Add a server')),
			E('p', {}, _('Paste the fiend:// link that you got from your server panel.')),
			input,
			E('div', { style: 'margin-top:8px' }, button)
		]);
	},

	render(data) {
		api.style();
		const [ initial, profiles, settings ] = data;
		if (!profiles.length && initial)
			return E('div', {}, [ E('h2', {}, _('Fiend')), this.importCard() ]);

		const dot = E('span', { class: 'fiend-dot' });
		const title = E('span');
		const detail = E('div', { class: 'fiend-muted', style: 'margin-top:4px' });
		const select = E('select', { class: 'cbi-input-select' }, profiles.map((profile) =>
			E('option', { value: profile.id }, profile.name)));
		const action = E('button', { class: 'btn cbi-button' });
		const notice = E('div', { class: 'alert-message warning', style: 'display:none' }, [
			E('span', {}, _('The settings changed. Reconnect to use them.')), ' ',
			E('button', {
				class: 'btn cbi-button cbi-button-action',
				click: (ev) => api.busy(ev.target, api.request('connect', { profile_id: select.value }))
					.catch((error) => api.fail(error))
			}, _('Reconnect'))
		]);
		const [ downTile, down ] = this.tile(_('Download'));
		const [ upTile, up ] = this.tile(_('Upload'));
		const [ receivedTile, received ] = this.tile(_('Received'));
		const [ sentTile, sent ] = this.tile(_('Sent'));
		const [ flowsTile, flows ] = this.tile(_('Connections'));
		const [ latencyTile, latency ] = this.tile(_('Latency'));

		const tiles = E('div', { class: 'fiend-tiles' }, [ downTile, upTile, receivedTile, sentTile, flowsTile, latencyTile ]);
		let connected = false;
		const update = (status) => {
			const state = api.state(status);
			dot.className = 'fiend-dot ' + state.dot;
			title.textContent = state.text;
			const active = status && status.state.state != 'disconnected' && status.state.state != 'failed';
			connected = !!active;
			action.textContent = active ? _('Disconnect') : _('Connect');
			action.className = 'btn cbi-button ' + (active ? 'cbi-button-negative' : 'cbi-button-positive');
			action.disabled = !status;
			if (status && status.profile_id && document.activeElement != select)
				select.value = status.profile_id;
			select.disabled = !!active;
			const lines = [];
			if (state.reason)
				lines.push(E('span', { class: 'fiend-error' }, state.reason));
			if (status && status.state.state == 'connected')
				lines.push(_('Connected for %s').format(api.duration(Date.now() - status.state.since_ms)));
			if (status && status.state.state == 'connected' && status.stats && status.stats.latency_ms == null)
				lines.push(E('span', { class: 'fiend-error' }, _('The server does not answer. Fiend tries again.')));
			dom.content(detail, lines.length ? lines.reduce((all, line) => all.concat(all.length ? [ ' · ', line ] : [ line ]), []) : '');
			notice.style.display = status && status.restart_required ? '' : 'none';
			const stats = status && status.stats;
			tiles.style.display = stats ? '' : 'none';
			down.textContent = stats ? api.rate(stats.download_bps) : '–';
			up.textContent = stats ? api.rate(stats.upload_bps) : '–';
			received.textContent = stats ? api.bytes(stats.received_bytes) : '–';
			sent.textContent = stats ? api.bytes(stats.sent_bytes) : '–';
			flows.textContent = stats ? String(stats.active_flows) : '–';
			latency.textContent = stats && stats.latency_ms != null ? '%d ms'.format(stats.latency_ms) : '–';
		};

		action.addEventListener('click', () => {
			const request = connected
				? api.request('disconnect')
				: api.request('connect', { profile_id: select.value });
			api.busy(action, request.then(() => api.status()).then(update)).catch((error) => api.fail(error));
		});

		if (settings && settings.active_profile)
			select.value = settings.active_profile;
		update(initial);
		poll.add(() => api.status().then(update).catch(() => update(null)), 2);

		const lists = settings ? settings.subscriptions.length : 0;
		const unlisted = settings && settings.default_route == 'tunnel'
			? _('All traffic goes through the tunnel, except Direct rules.')
			: _('Only the traffic in your lists goes through Fiend. All other traffic goes directly.');

		return E('div', {}, [
			E('h2', {}, _('Fiend')),
			notice,
			E('div', { class: 'fiend-card' }, [
				E('div', { class: 'fiend-row' }, [
					E('div', { class: 'fiend-grow' }, [ E('div', { class: 'fiend-state' }, [ dot, title ]), detail ]),
					profiles.length > 1 ? select : E('span', { class: 'fiend-muted' }, profiles[0] ? profiles[0].name : ''),
					action
				])
			]),
			tiles,
			E('div', { class: 'fiend-card' }, [
				E('div', { class: 'fiend-row' }, [
					E('div', { class: 'fiend-grow' }, [
						E('strong', {}, _('Routing')),
						E('div', { class: 'fiend-muted' }, [ unlisted, ' ', lists ? _('Lists: %d.').format(lists) : _('You have no lists yet.') ])
					]),
					E('a', { class: 'btn cbi-button', href: L.url('admin/services/fiend/routing') }, _('Edit routing'))
				])
			])
		]);
	},

	handleSave: null,
	handleSaveApply: null,
	handleReset: null
});
