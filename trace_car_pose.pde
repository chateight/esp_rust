/*
read m5stack udp data(IMU data ; pitch, roll, yaw) and move the imaginary object

*/
import hypermedia.net.*;

UDP udp;
final String IP = "0.0.0.0";

PFont myFont;

int[] pry = new int[4];


void setup() {
  
  udp = new UDP(this, 5001);
  udp.listen( true );

  size(600,600,P3D);
  frameRate(30);
  loop();
}

void draw() {
  background(0);

   pry[0] = pry[0] % 360;
   if (pry[0] < 0) {
    pry[0] += 360; // マイナスになったら360を足して正の角度にする（例：-45度 → 315度）
   }
 
  translate(width/2, height/2);

  if (pry[0] > 315 || pry[0] <= 45)
  {
  rotateX(radians(-20));
  rotateY(radians(0));
  rotateZ(radians(pry[0]));
  }
  else if (pry[0] > 45 && pry[0] <= 135)
  {
  rotateX(radians(0));
  rotateY(radians(-20));
  rotateZ(radians(pry[0]));
  }
  else if (pry[0] > 135 && pry[0] <= 225)
  {
  rotateX(radians(20));
  rotateY(radians(0));
  rotateZ(radians(pry[0]));
  }  
  else if (pry[0] > 225 && pry[0] <= 315)
  {
  rotateX(radians(0));
  rotateY(radians(20));
  rotateZ(radians(pry[0]));
  }
    drawFaceBox();
}

void drawFaceBox() {
  // 後ろの5面はグレー
  // cordination order x/y/z
  // notes  ↑ : -y, ↓ : +y
  //
  fill(200);
  beginShape(QUADS);
  vertex(-75, -75, -25);
  vertex(75, -75, -25);
  vertex(75, 75, -25);
  vertex(-75, 75, -25);
  endShape();

  fill(200);
  beginShape(QUADS);
  vertex(-75, -75, 25);
  vertex(75, -75, 25);
  vertex(75, 75, 25);
  vertex(-75, 75, 25);
  endShape();
  
  fill(255, 0, 0);
  beginShape(QUADS);
  vertex(-75, -75, -25);
  vertex(-75, -75, 25);
  vertex(75, -75, 25);
  vertex(75, -75, -25);
  endShape();
  
  fill(200);
  beginShape(QUADS);
  vertex(-75, 75, -25);
  vertex(-75, 75, 25);
  vertex(75, 75, 25);
  vertex(75, 75, -25);
  endShape();
  
  fill(200);
  beginShape(QUADS);
  vertex(75, -75, -25);
  vertex(75, -75, 25);
  vertex(75, 75, 25);
  vertex(75, 75, -25);
  endShape();
  
  fill(200);
  beginShape(QUADS);
  vertex(-75, -75, -25);
  vertex(-75, -75, 25);
  vertex(-75, 75, 25);
  vertex(-75, 75, -25);
  endShape();
}

void receive( byte[] data, String ip, int port ) {
  String message = new String( data );
  println( "received : \""+message+"\" from "+ip+" on port "+port );
  
  // カンマで文字列を分割し、文字列の配列にする
  String[] parts = split(message, ',');

  if (parts.length > 0) {
    String firstBlockString = parts[0];

    try {
      int firstBlockValue = int(firstBlockString);
      println("最初のブロックの数字: " + (firstBlockValue*360)/8900);

      // 必要であれば、pry配列の特定の要素に格納
      pry[0] = (firstBlockValue*360)/8900;

    } catch (NumberFormatException e) {
      // 最後のブロックが有効な数値ではなかった場合の処理
      println("エラー: 最後のブロック '" + firstBlockString + "' は数値に変換できません。");
    }
  } else {
    println("エラー: 受信したメッセージにはカンマ区切りが含まれていません。");
  }
}
