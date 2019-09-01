FROM node:12.9.1-alpine

WORKDIR /teatro/

COPY package*.json /teatro/
RUN npm i

CMD node .
COPY . /teatro/
