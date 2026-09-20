FROM ubuntu
RUN apt-get update && apt-get install -y curl vim
ADD config.json /app/config.json
COPY app.py /app/app.py
CMD ["python3", "/app/app.py"]
