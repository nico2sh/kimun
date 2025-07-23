#!/bin/sh -x

RESOLUTIONS="
    16,16x16
    32,16x16@2x
    32,32x32
    64,32x32@2x
    128,128x128
    256,128x128@2x
    256,256x256
    512,256x256@2x
    512,512x512
    1024,512x512@2x
"

for PNG in $@; do
  BASE=$(basename "$PNG" | sed 's/\.[^\.]*$//')
    ICONSET="$BASE.iconset"
    ICONSET_DIR="./$ICONSET"
    mkdir -p "$ICONSET_DIR"
    for RES in ${RESOLUTIONS[@]}; do
        SIZE=$(echo $RES | cut -d, -f1)
        LABEL=$(echo $RES | cut -d, -f2)
        magick "$PNG" -resize $SIZEx$SIZE "$ICONSET_DIR"/icon_$LABEL.png
    done

    iconutil -c icns "$ICONSET_DIR"
done
