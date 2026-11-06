'use strict';
'require view';
'require poll';
'require dom';
'require fiend.api as api';

const LIMIT = 500;

return view.extend({
	load() {
		return api.logs(LIMIT).catch(() => []);
	},

	line(entry) {
		const time = new Date(entry.timestamp_ms).toLocaleTimeString();
		return E('div', { class: entry.level }, '%s %s %s'.format(time, entry.level.toUpperCase().padEnd(5), entry.message));
	},

	render(entries) {
		api.style();
		const box = E('div', { class: 'fiend-log' });
		let last = null;
		const show = (list) => {
			const key = list.length ? list[list.length - 1].timestamp_ms + list[list.length - 1].message : '';
			if (key == last)
				return;
			last = key;
			const bottom = box.scrollHeight - box.scrollTop - box.clientHeight < 40;
			dom.content(box, list.length ? list.map((entry) => this.line(entry)) : E('div', { class: 'fiend-muted' }, _('The log is empty.')));
			if (bottom)
				box.scrollTop = box.scrollHeight;
		};
		show(entries);
		requestAnimationFrame(() => box.scrollTop = box.scrollHeight);
		poll.add(() => api.logs(LIMIT).then(show).catch(() => {}), 3);

		const clear = E('button', {
			class: 'btn cbi-button',
			click: () => api.busy(clear, api.request('clear_logs')).then(() => show([])).catch((error) => api.fail(error))
		}, _('Clear'));

		return E('div', {}, [
			E('div', { class: 'fiend-row' }, [ E('h2', { class: 'fiend-grow' }, _('Log')), clear ]),
			box
		]);
	},

	handleSave: null,
	handleSaveApply: null,
	handleReset: null
});
