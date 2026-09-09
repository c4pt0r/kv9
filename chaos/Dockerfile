FROM ubuntu:24.04 AS fixture-builder
RUN apt-get update && apt-get install -y --no-install-recommends gcc libc6-dev \
    && rm -rf /var/lib/apt/lists/*
COPY chaos/no-statx.c /tmp/no-statx.c
RUN gcc -O2 -Wall -Wextra -Werror /tmp/no-statx.c -o /usr/local/bin/no-statx

FROM ubuntu:24.04

COPY target/debug/kv9 /usr/local/bin/kv9
COPY target/debug/examples/admission-pressure /usr/local/bin/admission-pressure
COPY target/chaos-persistent/kv9-workload /usr/local/bin/kv9-workload
COPY target/chaos-persistent/build.json /opt/kv9-workload/build.json
COPY --from=fixture-builder /usr/local/bin/no-statx /usr/local/bin/no-statx

ENTRYPOINT ["/usr/local/bin/kv9"]
