FROM node:12.10.0-alpine

WORKDIR /teatro/

COPY package*.json /teatro/
RUN npm i

CMD node .
COPY . /teatro/
