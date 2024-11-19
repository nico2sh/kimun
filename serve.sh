#!/usr/bin/env bash

# npx tailwindcss -i ./input.css -o ./public/tailwind.css --watch &
# dx serve --platform desktop && fg

(
  trap 'kill 0' SIGINT
  npx tailwindcss -i ./input.css -o ./public/tailwind.css --watch &
  dx serve --hot-reload --platform desktop &
  wait
)
