window.addEventListener("beforeunload", (event) => {
	client.end();
});

//name -> observers id
var undefined, attrs = {};

function isNumeric(n) {
	return !isNaN(parseFloat(n)) && isFinite(n);
}

function send_attr_value(attr, value, as_str) {
	var undefined;

	if(as_str === undefined)
	{
		//auto
		if(!(value === 'true' || value === 'false' || isNumeric(value)))
			as_str = true;
	}

	if(as_str)
		value = '"' + value + '"';

	var s = attr.split('.');

	if(s.length == 2)
		client.publish('devices/' + s[0] + '/store/' + s[1], value);
}

window.addEventListener("load", (event) => {
	const client = mqtt.connect("ws://" + location.host + "/switcher/mqtt");

	client.on("message", function(topic, payload) {
		var s = topic.split('/');

		if(s.length == 4 && s[0] == 'devices' && s[2] == 'actual')
		{
			var attr = s[1] + '.' + s[3];
			var value = payload.toString();

			if(attrs[attr])
			{
				for(let oid of attrs[attr]) {
					switch(oid.type)
					{
						case 'button':
							var oel = document.getElementById("attr_" + oid.counter);

							if(value == "true")
								oel.classList.add('active');
							else
								oel.classList.remove('active');

							break;

						case 'switch':
						case 'switch_ro':
							var oel = document.getElementById("attr_" + oid.counter);

							if(oel.last_timeout)
								clearTimeout(oel.last_timeout);

							oel.last_checked = oel.children[0].checked;
							oel.children[0].checked = value == "true";
							break;

						case 'text':
							var oel = document.getElementById("attr_" + oid.counter);

							if(oid.e && oid.e.round)
							{
								value = Number.parseFloat(value).toFixed(oid.e.round);

								if(navigator.language)
									value = Intl.NumberFormat(navigator.language).format(value);
							}

							if(oid.e && oid.e.unit)
								value += ' ' + oid.e.unit;

							oel.innerHTML = value;
							break;

						case 'select':
							var oel = document.getElementById("attr_" + oid.counter);

							if(oel.last_timeout)
								clearTimeout(oel.last_timeout);

							oel.last_value = oel.value;
							oel.value = value;
							break;

						case 'range':
							var oel = document.getElementById("attr_" + oid.counter);

							if(oel.last_timeout)
								clearTimeout(oel.last_timeout);

							oel.last_value = oel.value;
							oel.value = parseInt(value);
							break;

						case 'rgb':
							var oel = document.getElementById("attr_" + oid.counter);
							oel.value = '#' + value.substring(0, 6);
							break;

						//props
						case 'reachable':
						case 'signal':
						case 'battery':
							var panel = document.getElementById("panel_" + oid.counter);
							var prop = panel.querySelector('.props .' + oid.type);

							if(prop)
								prop.dataset[oid.type] = value;

							break;
					}
				}
			}
		}
	});

	window.client = client;

	//observer counter
	var counter = 0;

	//topics to subscribe
	var subscribe_attrs = [];

	function get_ctrl_html(e)
	{
		counter += 1;

		var h = '<div class="controll-wrapper"';

		if(e.role)
			h += ' data-role="' + e.role + '"';

		if(e.role)
			h += ' data-type="' + e.type + '"';

		h += '>';

		if(e.title)
			h += '<div class="title">' + e.title + '</div>';

		h += '<div class="controller">';

		switch(e.type)
		{
			case 'select':
				h += '<select id="attr_' + counter + '">';

				for(var k in e.value_map)
					h += '<option value="' + k + '">' + e.value_map[k] + '</option>';

				h += '</select>';
				break;

			case 'button':
				h += '<button class="button" id="attr_' + counter + '"></button>';
				break;

			case 'switch':
				h += '<label class="switch" id="attr_' + counter + '"><input type="checkbox"><span class="slider"></span></label>';
				break;

			case 'switch_ro':
				h += '<label class="switch ro" id="attr_' + counter + '"><input type="checkbox" disabled="disabled"><span class="slider"></span></label>';
				break;

			case 'text':
				h += '<label id="attr_' + counter + '"></label>';
				break;

			case 'range':
				h += '<input id="attr_' + counter + '" type="range" min="' + e.min + '" max="' + e.max + '">';
				break;

			case 'rgb':
				h += '<input id="attr_' + counter + '" type="color">';
				break;
		}

		h += '</div></div>';

		if(!attrs[e.attr])
			attrs[e.attr] = [];

		attrs[e.attr].push({counter, type: e.type, e});

		subscribe_attrs.push(e.attr);

		return h;
	}

	window.icon_click = function(e)
	{
		var n = e.parentNode;
		n.classList.toggle('full-screen');

		var is_full = n.classList.contains('full-screen');

		//hide titles
		document.getElementById('swiper-titles').style.display = is_full ? 'none' : null;

		//resize parent to full screen
		document.getElementById('swiper-pages').style.height = is_full ? '100%' : null;
	}

	function isTouchDevice() {
		return ('ontouchstart' in window) || navigator.maxTouchPoints > 0 || navigator.msMaxTouchPoints > 0;
	}

	function get_element_html(e)
	{
		counter += 1;
		let type = e.type;

		let h = '<div class="sw-dev-panel" id="panel_' + counter + '"';

		var display = e.display ? e.display : 'list';

		h += ' data-display="' + display + '"';

		if(e.icon)
			h += ' data-icon="' + e.icon + '"';

		h += '>';

		if(e.icon)
			h += '<div class="icon" onclick="icon_click(this);"></div>';

		if(e.props)
		{
			var ph = '';

			for(var prop in e.props)
			{
				var prop_attr = e.props[prop];

				if(!attrs[prop_attr])
					attrs[prop_attr] = [];

				attrs[prop_attr].push({counter, type: prop});
	
				subscribe_attrs.push(prop_attr);

				ph += '<div class="' + prop + '"></div>';
			}

			if(ph)
				h += '<div class="props">' + ph + '</div>';
		}

		if(e.title)
			h += '<div class="title">' + e.title + '</div>';

		if(e.controls)
		{
			h += '<div class="sw-controls">';

			for(let el of e.controls)
				h += get_ctrl_html(el);

			h += "</div>";
		}

		h += "</div>";

		return h;
	}

	if(window.system)
	{
		for(let p of system.pages) {
			let h = '<div class="swiper-slide"><div class="sw-dev-container">';

			if(p.elements)
				for(let e of p.elements)
					h += get_element_html(e);

			h += '</div></div>';

			document.getElementById('pages').innerHTML += h;
			document.getElementById('titles').innerHTML += '<div class="swiper-slide">' + p.title + '</div>';
		}

		//round range sliders
		const elements = document.querySelectorAll('[data-range]');

		elements.forEach(element => {
			new RangeSlider(element, element.dataset);
		})

		//attach event listeners
		for(var attr in attrs)
		{
			var elems = attrs[attr];

			for(var elem of elems)
			{
				var id = 'attr_' + elem.counter;
				var he = document.getElementById(id);

				if(he)
				{
					he.sw_attr = {attr: attr, counter: elem.counter};

					switch(elem.type)
					{
						case 'select':
						case 'range':
							he.onchange = function(e){
								var value = this.value;

								var last_value = this.last_value;
								this.last_timeout = setTimeout(() => {
									this.value = last_value;
								}, 2000);

								send_attr_value(this.sw_attr.attr, value);
							};
							break;

						case 'button':
							if(isTouchDevice())
							{
								he.ontouchstart = function(e){send_attr_value(this.sw_attr.attr, 'true');};
								he.ontouchend = function(e){send_attr_value(this.sw_attr.attr, 'false');};
							}
							else
							{
								he.onmousedown = function(e){send_attr_value(this.sw_attr.attr, 'true');};
								he.onmouseup = function(e){send_attr_value(this.sw_attr.attr, 'false');};
							}
							break;

						case 'switch':
							he.onchange = function(e){
								var checked = this.children[0].checked;

								var last_checked = this.last_checked;
								this.last_timeout = setTimeout(() => {

									this.children[0].checked = last_checked;
								}, 2000);

								send_attr_value(this.sw_attr.attr, checked ? 'true' : 'false');
							};
							break;

						case 'rgb':
							//force send val as string avoiding converting to integer
							he.onchange = function(e){
								send_attr_value(this.sw_attr.attr, this.value.substring(1), true);
							};
							break;
					}
				}
			}
		}
	}

	var subscribe_topics = subscribe_attrs.reduce(function(filtered, attr){
		var s = attr.split('.');

		if(s.length == 2)
			filtered.push('devices/' + s[0] + '/actual/' + s[1]);

		return filtered;
	}, []);

	if(subscribe_topics.length > 0)
		client.subscribe(subscribe_topics);

	const swiper_titles = new Swiper('#swiper-titles', {
		observer: true,
		spaceBetween: 0,
		loop: true,
		freeMode: true,
		slidesPerView: 'auto',
	});

	const swiper = new Swiper('#swiper-pages', {
		observer: true,
		spaceBetween: 0,
		loop: true,
		thumbs: {
			swiper: swiper_titles,
		}
	});
});
