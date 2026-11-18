FROM nvidia/cuda:12.9.1-cudnn-runtime-ubuntu24.04

RUN apt update && apt -y install python3 python3-venv

WORKDIR /app

COPY ./requirements.txt ./requirements-torch.txt /app/

RUN python3 -m venv venv && . venv/bin/activate && \
    pip install -r requirements.txt && pip install -r requirements-torch.txt

COPY ./transcript_cli.py ./model_cache.py /app/

ENTRYPOINT ["./venv/bin/python", "transcript_cli.py"]
