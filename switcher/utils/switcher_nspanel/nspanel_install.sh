#https://stackoverflow.com/questions/30998343/how-to-set-selinux-to-0-or-permissive-mode-in-android-4-4-4-and-above/32660547#32660547

mount -o remount,rw /system
mkdir /system/su.d
echo "#!/system/bin/sh" > /system/su.d/switcher.sh
#echo "/opt/switcher/run.sh" >> /system/su.d/switcher.sh
echo "/opt/switcher/switcher /opt/switcher/config.json" >> /system/su.d/switcher.sh
chmod 755 /system/su.d/switcher.sh
chown 0.0 /system/su.d/switcher.sh
