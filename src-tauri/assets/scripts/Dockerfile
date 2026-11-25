FROM docker.io/nvidia/cuda:12.9.2-cudnn-runtime-ubuntu24.04

RUN apt-get update && apt-get install -y --no-install-recommends \
    libgomp1 python3 python3-venv && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY ./requirements.txt /app/

RUN python3 -m venv venv && \
    venv/bin/python -m pip install --no-cache-dir -r requirements.txt && \
    venv/bin/python -m pip check && \
    venv/bin/python -c 'from ctranslate2 import get_cuda_device_count; from faster_whisper import BatchedInferencePipeline, WhisperModel, format_timestamp'

COPY ./transcript_cli.py ./model_cache.py /app/

ENTRYPOINT ["./venv/bin/python", "transcript_cli.py"]
