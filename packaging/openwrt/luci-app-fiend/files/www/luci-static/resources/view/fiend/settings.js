'use strict';
'require view';
'require form';
'require uci';
'require ui';
'require tools.widgets as widgets';
'require fiend.api as api';

return view.extend({
	load() {
		return Promise.all([ uci.load('fiend'), api.settings() ]);
	},

	render(data) {
		const settings = api.settingsOf(data[1]);
		this.settings = settings;

		const router = new form.Map('fiend', _('Settings'));
		let s = router.section(form.NamedSection, 'main', 'fiend', _('Router'));
		let o = s.option(widgets.NetworkSelect, 'network', _('Networks'),
			_('Fiend takes the traffic of the devices in these networks.'));
		o.multiple = true;
		o.nocreate = true;
		o.rmempty = false;
		o = s.option(form.Flag, 'dns_redirect', _('Catch all DNS queries'),
			_('Send the DNS queries of the devices to the router, also when a device uses a different DNS server. Without this, lists of sites do not work for such devices.'));
		o.default = '1';
		o.rmempty = false;

		this.data = {
			tunnel: {
				connect_at_startup: settings.connect_at_startup ? '1' : '0',
				ipv6: settings.ipv6 ? '1' : '0',
				direct_resolver: settings.direct_resolver
			}
		};
		const tunnel = new form.JSONMap(this.data);
		s = tunnel.section(form.NamedSection, 'tunnel', 'tunnel', _('Tunnel'));
		o = s.option(form.Flag, 'connect_at_startup', _('Connect at startup'),
			_('Connect when the router starts.'));
		o = s.option(form.Flag, 'ipv6', _('IPv6'),
			_('Send IPv6 traffic through Fiend. When off, Fiend gives no IPv6 addresses for the sites in your lists, so that devices use IPv4.'));
		o = s.option(form.Value, 'direct_resolver', _('DNS server'),
			_('The server for the DNS queries that do not go through the tunnel. Empty: the server that the router used before Fiend.'));
		o.placeholder = _('The DNS server of the router');
		o.datatype = 'or(ipaddr,ipaddrport(1))';

		this.maps = [ router, tunnel ];
		return Promise.all(this.maps.map((map) => map.render())).then((nodes) => E('div', {}, nodes));
	},

	saveTunnel() {
		const values = this.data.tunnel;
		return api.settings().then((reply) => {
			const settings = api.settingsOf(reply);
			settings.connect_at_startup = values.connect_at_startup == '1';
			settings.ipv6 = values.ipv6 == '1';
			settings.direct_resolver = (values.direct_resolver || '').trim();
			return api.request('set_settings', { settings: settings });
		});
	},

	handleSave() {
		return Promise.all(this.maps.map((map) => map.save()))
			.then(() => this.saveTunnel())
			.catch((error) => api.fail(error));
	},

	handleSaveApply(ev, mode) {
		return this.handleSave().then(() => ui.changes.apply(mode == '0'));
	}
});
