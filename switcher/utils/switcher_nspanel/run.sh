#!/system/bin/sh

#nohup /opt/switcher/switcher /opt/switcher/config.json >/dev/null 2>&1
#hwclock -s -u

while true
do
    /opt/switcher/switcher /opt/switcher/config.json
    sleep 1
done
