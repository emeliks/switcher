rm -r switcher_http/.svn

adb shell su -c mount -o remount,rw /
adb shell su -c mount -o remount,rw /vendor
adb shell su -c mount -o remount,rw /system
adb shell su -c mkdir /opt/switcher

adb push switcher_http /opt/switcher
adb push switcher /opt/switcher
adb push config.json /opt/switcher
adb push run.sh /opt/switcher

adb push nspanel_install.sh /opt/switcher
adb shell su -c /opt/switcher/nspanel_install.sh


