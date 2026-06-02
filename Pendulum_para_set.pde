/*
set PID parametor to the m5stack using UDP

*/
import hypermedia.net.*;
import controlP5.*;

UDP udp;
ControlP5 cp5;

final String IP = "192.168.1.12";
final int PORT = 3002;
String msg = "1, 10, 0.05";  // P, I, D

void setup() {
  size(200, 200);

  cp5 = new ControlP5(this);
  udp = new UDP( this, 3002 );

  ControlFont cf = new ControlFont(createFont("Serif",20));

  cp5.addButton("UDP_Msg")
    .setFont(cf)
    .setLabel("send")
    .setPosition(50,50)
    .setSize(100,100);
}

void draw() {
  background(200);
}

void UDP_Msg(){
  udp.send(msg, IP, PORT);
}
