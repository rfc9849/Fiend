'use strict';
'require view';
'require dom';
'require ui';
'require fiend.api as api';

return view.extend({
	load() {
		return Promise.all([ api.profiles(), api.settings(), api.status().catch(() => null) ]);
	},

	reload() {
		return this.load().then((data) => dom.content(this.root, this.body(data)));
	},

	remove(profile) {
		ui.showModal(_('Delete profile'), [
			E('p', {}, _('Delete the profile "%s"?').format(profile.name)),
			E('div', { class: 'right' }, [
				E('button', { class: 'btn cbi-button', click: ui.hideModal }, _('Cancel')), ' ',
				E('button', {
					class: 'btn cbi-button cbi-button-negative',
					click: (ev) => api.busy(ev.target, api.request('delete_profile', { id: profile.id }))
						.then(() => { ui.hideModal(); return this.reload(); })
						.catch((error) => { ui.hideModal(); api.fail(error); })
				}, _('Delete'))
			])
		]);
	},

	item(profile, settings, status) {
		const active = settings.active_profile == profile.id;
		const running = status && status.profile_id == profile.id && status.state.state != 'disconnected';
		const latency = E('span');
		const test = E('button', {
			class: 'btn cbi-button',
			click: () => api.busy(test, api.request('measure_latency', { profile_id: profile.id }))
				.then((reply) => latency.textContent = ' · ' + _('%d ms').format(reply.ms))
				.catch((error) => {
					latency.textContent = '';
					dom.content(latency, [ ' · ', E('span', { class: 'fiend-error' }, error.message) ]);
				})
		}, _('Test'));
		const use = E('button', {
			class: 'btn cbi-button cbi-button-action',
			disabled: running ? '' : null,
			click: () => api.busy(use, api.request('connect', { profile_id: profile.id }))
				.then(() => this.reload())
				.catch((error) => api.fail(error))
		}, running ? _('In use') : _('Connect'));
		return E('div', { class: 'fiend-item' }, [
			E('div', { class: 'fiend-grow' }, [
				E('div', { class: 'title' }, [ profile.name, active ? E('span', { class: 'fiend-muted' }, ' · ' + _('active')) : '' ]),
				E('div', { class: 'meta' }, [ profile.servers.join(', '), latency ])
			]),
			test, use,
			E('button', { class: 'btn cbi-button cbi-button-negative', click: () => this.remove(profile) }, _('Delete'))
		]);
	},

	body(data) {
		const [ profiles, settings, status ] = data;
		const input = E('textarea', { class: 'cbi-input-textarea', rows: 3, style: 'width:100%', placeholder: 'fiend://…' });
		const add = E('button', {
			class: 'btn cbi-button cbi-button-positive',
			click: () => api.busy(add, api.addProfile(input.value))
				.then(() => this.reload())
				.catch((error) => api.fail(error))
		}, _('Add profile'));
		return [
			E('div', { class: 'fiend-list' }, profiles.length
				? profiles.map((profile) => this.item(profile, settings, status))
				: E('div', { class: 'fiend-empty' }, _('You have no profiles yet.'))),
			E('div', { class: 'fiend-card' }, [
				E('h3', {}, _('Add a profile')),
				E('p', {}, _('Paste the fiend:// link that you got from your server panel.')),
				input,
				E('div', { style: 'margin-top:8px' }, add)
			])
		];
	},

	render(data) {
		api.style();
		this.root = E('div', {}, this.body(data));
		return E('div', {}, [ E('h2', {}, _('Profiles')), this.root ]);
	},

	handleSave: null,
	handleSaveApply: null,
	handleReset: null
});
