'use strict';
'require view';
'require form';
'require uci';
'require fs';
'require ui';

return view.extend({
	render: function () {
		var m, s, o;

		m = new form.Map('diplay', _('DiPlay Cast'),
			_('Headless CarPlay receiver streaming to a DLNA-only car screen. '
			+ 'CarPlay audio/video is received from the iPhone and pushed as an MPEG-TS stream '
			+ 'to the TV/head unit DLNA receiver. Touch is not supported on the car screen.'));

		s = m.section(form.NamedSection, 'main', 'diplay', _('Settings'));

		o = s.option(form.Flag, 'enabled', _('Enabled'));
		o.default = '1';

		o = s.option(form.ListValue, 'connection', _('iPhone connection'));
		o.value('wireless', _('Wireless (Bluetooth + Wi-Fi)'));
		o.value('wired', _('Wired (USB Host)'));
		o.default = 'wireless';

		s.option(form.Value, 'wifi_ssid', _('Hotspot SSID'));
		s.option(form.Value, 'wifi_key', _('Hotspot password'));
		o = s.option(form.Value, 'wifi_country', _('Wi-Fi country'));
		o.default = 'CN';
		o = s.option(form.Value, 'wifi_channel', _('Wi-Fi channel'));
		o.default = '6';

		s.option(form.Flag, 'cast_enabled', _('DLNA cast output'));
		o = s.option(form.Value, 'cast_width', _('Video width'));
		o.default = '1280';
		o = s.option(form.Value, 'cast_height', _('Video height'));
		o.default = '720';
		o = s.option(form.ListValue, 'cast_fps', _('Frame rate'));
		o.value('30', '30 fps');
		o.value('60', '60 fps');
		o.default = '30';
		o = s.option(form.Value, 'cast_udn', _('Renderer UDN (optional)'));
		o.description = _('Leave empty to use the first DLNA MediaRenderer found.');
		s.option(form.Value, 'cast_interface', _('LAN interface IPv4 (optional)'));
		s.option(form.Flag, 'cast_no_audio', _('Disable streamed audio'));
		o = s.option(form.Value, 'cast_title', _('Renderer title'));
		o.default = 'CarPlay';

		o = s.option(form.TextValue, '_identity_status', _('MFi identity status'));
		o.readonly = true;
		o.render = function (section_id, value) {
			return fs.list('/etc/diplay/identity').then(function (files) {
				var names = Object.keys(files || {});
				var has_pk8 = names.indexOf('identity.pk8') >= 0;
				var has_p7b = names.indexOf('certificate.p7b') >= 0;
				var ok = has_pk8 && has_p7b;
				return E('div', {}, [
					E('p', {}, ok
						? _('MFi identity files present. Do not distribute this firmware or these keys.')
						: _('MFi identity missing: place identity.pk8 and certificate.p7b in /etc/diplay/identity/ (mode 0600).')),
					E('pre', {}, names.join('\n') || _('(empty)'))
				]);
			});
		};

		return m.render();
	}
});
